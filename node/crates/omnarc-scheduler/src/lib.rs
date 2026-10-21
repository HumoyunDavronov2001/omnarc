//! The placement of an Omnarc cluster (PLAN §5): which device holds which layers of a model.
//!
//! A plan is solved as a mixed-integer program with `HiGHS`, after the Halda formulation of `prima.cpp`: devices
//! stand in ring order, each holds a contiguous window of layers, and the objective is the time one token
//! takes around the ring. It is extended with the rules of REQUIREMENTS Q1, Q7 and R6:
//!
//! - what a device offers is a hard limit of the plan, never a suggestion (NO-51);
//! - a device too small for any window is left out rather than slowing the ring (NO-52);
//! - a model that fits on one device is not split at all; the other devices that can hold it get whole
//!   copies (NO-44).
//!
//! Every stage also gets a standby replica where one fits (EN-17), and [`backups`] precomputes the plan for
//! each device leaving, so a departure is answered without solving anything (NO-43).

use std::fmt;
use std::ops::Range;
use std::time::Duration;

use highs::{HighsModelStatus, RowProblem, Sense};

/// What the plan needs to know about a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    pub layers: u32,
    /// Weights of one repeating layer.
    pub layer_bytes: u64,
    /// KV cache of one layer at the context the plan is made for.
    pub kv_bytes_per_layer: u64,
    /// The token embedding, held by the head: the first stage of the ring.
    pub head_bytes: u64,
    /// The output norm and head, held by the head too: the last stage sends its hidden states back around, the head
    /// turns them into the next token and feeds it in again.
    pub tail_bytes: u64,
}

impl Model {
    /// The memory a window of `layers` takes; the head's holds the embedding and the output too.
    #[must_use]
    pub fn window_bytes(&self, layers: u32, head: bool) -> u64 {
        u64::from(layers) * (self.layer_bytes + self.kv_bytes_per_layer)
            + if head { self.head_bytes + self.tail_bytes } else { 0 }
    }

    /// Everything the model takes on one device.
    #[must_use]
    pub fn whole_bytes(&self) -> u64 {
        self.window_bytes(self.layers, true)
    }
}

/// What a device brings to the plan: what its owner offers, and what the profiler measured.
#[derive(Debug, Clone, PartialEq)]
pub struct Device<K> {
    pub id: K,
    /// Memory the owner offers right now (Q1); the plan never uses more.
    pub offered_bytes: u64,
    /// Time this device takes for one layer of one token, measured by the profiler (NO-40).
    pub seconds_per_layer: f64,
    /// Time a token's hidden state takes from this device to the next one in the ring.
    pub link_seconds: f64,
    /// The one GPU the device lends layers to, when it has one worth using (NO-40, NO-51).
    pub gpu: Option<Gpu>,
    /// Share of the model's layers the device already holds, 0..=1: those need no fetching.
    pub held: f64,
    /// Time one byte of weights it lacks takes to reach it from the best device that has them; 0 when it holds all.
    pub fetch_seconds_per_byte: f64,
}

/// A fetch is weighed as spread over the plan's first tokens: moving gigabytes over a slow link costs the first
/// answers far more than a slightly slower ring does.
pub const AMORTIZED_TOKENS: f64 = 1000.0;

// The time `bytes` of the model's weights take to reach `d`, the part it already holds left out.
#[allow(clippy::cast_precision_loss)]
fn fetch_seconds<K>(d: &Device<K>, bytes: u64) -> f64 {
    (1.0 - d.held.clamp(0.0, 1.0)) * bytes as f64 * d.fetch_seconds_per_byte
}

/// A GPU of a device: what its owner lets the plan use of it, and how fast it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gpu {
    /// GPU memory the owner offers (Q1).
    pub offered_bytes: u64,
    /// Time this GPU takes for one layer of one token (NO-40).
    pub seconds_per_layer: f64,
    /// An integrated GPU: its memory is the device's own, so it adds speed but no room.
    pub shares_memory: bool,
}

/// One device's window of layers, in ring order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage<K> {
    pub node: K,
    pub layers: Range<u32>,
    /// How many of the window's layers run on the device's GPU, the last ones; on the head the output goes along.
    pub gpu_layers: u32,
}

/// A standby copy of a stage's window on another device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replica<K> {
    pub node: K,
    pub layers: Range<u32>,
    /// As in [`Stage`].
    pub gpu_layers: u32,
}

/// Why a device has no part in a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftOut {
    /// It offers less than one layer needs (NO-52).
    TooSmall,
    /// The model runs faster without it (R6).
    NotNeeded,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Placement<K> {
    /// Ring order; together the windows cover every layer once.
    pub stages: Vec<Stage<K>>,
    pub replicas: Vec<Replica<K>>,
    pub left_out: Vec<(K, LeftOut)>,
    /// The estimate the plan was chosen by: one token around the ring.
    pub seconds_per_token: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The devices together do not offer what the model needs.
    DoesNotFit { needed: u64, offered: u64 },
    /// The solver gave up within the time it was allowed.
    NoPlan,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DoesNotFit { needed, offered } => {
                #[allow(clippy::cast_precision_loss)]
                let gib = |bytes: u64| bytes as f64 / f64::from(1u32 << 30);
                write!(
                    f,
                    "it does not fit: the model needs {:.1} GiB and the devices offer {:.1} GiB together",
                    gib(*needed),
                    gib(*offered)
                )
            }
            Self::NoPlan => f.write_str("no plan was found in the time allowed"),
        }
    }
}

impl std::error::Error for Error {}

/// The most devices one placement solves at once (NO-41); larger networks are planned in groups of this size.
pub const GROUP: usize = 64;

// The GPU layers a window of `layers` gets on `d`, when it fits there at all. A GPU quicker than the processor
// takes as many layers as it holds; a slower one only what the processor's memory cannot. With any layer on the
// GPU the head's output goes along, as the engine does it.
fn fit<K>(model: &Model, d: &Device<K>, layers: u32, head: bool) -> Option<u32> {
    let per_layer = model.layer_bytes + model.kv_bytes_per_layer;
    let Some(gpu) = &d.gpu else {
        return (model.window_bytes(layers, head) <= d.offered_bytes).then_some(0);
    };
    let ends = if head { model.head_bytes + model.tail_bytes } else { 0 };
    let tail = if head { model.tail_bytes } else { 0 };
    let most = if gpu.offered_bytes >= tail + per_layer {
        u32::try_from((gpu.offered_bytes - tail) / per_layer).unwrap_or(u32::MAX).min(layers)
    } else {
        0
    };
    let memory = |g: u32| {
        if gpu.shares_memory {
            u64::from(layers) * per_layer + ends
        } else {
            u64::from(layers - g) * per_layer + ends - if g > 0 { tail } else { 0 }
        }
    };
    if gpu.seconds_per_layer < d.seconds_per_layer {
        (memory(most) <= d.offered_bytes).then_some(most)
    } else {
        (0..=most).find(|&g| memory(g) <= d.offered_bytes)
    }
}

// One token through a window of `layers` with `gpu` of them on the GPU.
fn window_seconds<K>(d: &Device<K>, layers: u32, gpu: u32) -> f64 {
    let on_gpu = d.gpu.map_or(0.0, |g| f64::from(gpu) * (g.seconds_per_layer - d.seconds_per_layer));
    f64::from(layers) * d.seconds_per_layer + on_gpu
}

// What a device holds, its discrete GPU counted, and its time per layer over all of that: for ranking devices.
#[allow(clippy::cast_precision_loss)]
fn capacity<K>(model: &Model, d: &Device<K>) -> (u64, f64) {
    let Some(gpu) = &d.gpu else { return (d.offered_bytes, d.seconds_per_layer) };
    let held = d.offered_bytes + if gpu.shares_memory { 0 } else { gpu.offered_bytes };
    if gpu.seconds_per_layer >= d.seconds_per_layer {
        return (held, d.seconds_per_layer);
    }
    let per_layer = (model.layer_bytes + model.kv_bytes_per_layer).max(1) as f64;
    let all = (held as f64 / per_layer).max(1.0);
    let fast = (gpu.offered_bytes as f64 / per_layer).min(all);
    (held, (fast * gpu.seconds_per_layer + (all - fast) * d.seconds_per_layer) / all)
}

// The group a large network is planned in: the fastest devices, as many as hold the model with room for the head,
// and no more than GROUP.
fn group<'a, K>(model: &Model, usable: &[&'a Device<K>]) -> Vec<&'a Device<K>> {
    let mut by_speed = usable.to_vec();
    by_speed.sort_by(|a, b| capacity(model, a).1.total_cmp(&capacity(model, b).1));
    // room for every window plus the head's extras on any one of them, with a margin for layers that do not divide
    let needed = model.whole_bytes() + (model.layer_bytes + model.kv_bytes_per_layer) * 2;
    let mut held = 0;
    let mut chosen = Vec::new();
    for d in by_speed {
        if chosen.len() == GROUP || (held >= needed && chosen.len() >= 2) {
            break;
        }
        held += capacity(model, d).0;
        chosen.push(d);
    }
    // the ring keeps the order the devices came in
    chosen.sort_by_key(|d| usable.iter().position(|u| std::ptr::eq(*u, *d)));
    chosen
}

/// The time `HiGHS` may take for one plan (W6: a plan for 64 devices within a second).
pub const TIME_LIMIT: Duration = Duration::from_millis(900);

/// Places `model` on `devices`, which stand in ring order.
///
/// # Errors
///
/// [`Error::DoesNotFit`] when no assignment stays within what the devices offer.
pub fn place<K: Clone>(model: &Model, devices: &[Device<K>]) -> Result<Placement<K>, Error> {
    place_for(model, devices, Objective::Latency)
}

/// What a plan is chosen for (NO-45).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Objective {
    /// One request as fast as it goes: the least time a token takes around the ring.
    #[default]
    Latency,
    /// Many requests at once: the ring is as fast as its slowest stage, so that one is made as quick as it can be.
    Throughput,
}

/// [`place`] for `objective`.
///
/// # Errors
///
/// As [`place`].
pub fn place_for<K: Clone>(model: &Model, devices: &[Device<K>], objective: Objective) -> Result<Placement<K>, Error> {
    let (usable, small): (Vec<_>, Vec<_>) = devices.iter().partition(|d| fit(model, d, 1, false).is_some());
    let mut left_out: Vec<_> = small.into_iter().map(|d| (d.id.clone(), LeftOut::TooSmall)).collect();
    let offered = || devices.iter().map(|d| capacity(model, d).0).sum();

    // R6: a model that fits on one device, its GPU counted, runs there, on the device that answers soonest — its
    // speed and, when it must first fetch the model, the fetch spread over the first tokens — and nowhere split
    let whole = model.whole_bytes();
    let alone = usable
        .iter()
        .filter_map(|d| {
            fit(model, d, model.layers, true)
                .map(|g| (*d, g, window_seconds(d, model.layers, g), fetch_seconds(d, whole) / AMORTIZED_TOKENS))
        })
        .min_by(|a, b| (a.2 + a.3).total_cmp(&(b.2 + b.3)));
    // a split is weighed against it only when that device would first fetch what others already hold
    let split = match alone {
        Some((.., 0.0)) => None,
        _ => Some(split_placement(model, &usable, objective)),
    };
    if let Some((best, gpu_layers, seconds, fetch)) = alone {
        let split_sooner = matches!(&split, Some(Ok((p, f))) if p.seconds_per_token + f < seconds + fetch);
        if !split_sooner {
            let layers = 0..model.layers;
            let mut replicas = Vec::new();
            for d in &usable {
                if std::ptr::eq(*d, best) {
                    continue;
                }
                match fit(model, d, model.layers, true) {
                    Some(g) => replicas.push(Replica { node: d.id.clone(), layers: layers.clone(), gpu_layers: g }),
                    None => left_out.push((d.id.clone(), LeftOut::NotNeeded)),
                }
            }
            return Ok(Placement {
                stages: vec![Stage { node: best.id.clone(), layers, gpu_layers }],
                replicas,
                left_out,
                seconds_per_token: seconds,
            });
        }
    }
    if usable.iter().map(|d| capacity(model, d).0).sum::<u64>() < whole {
        return Err(Error::DoesNotFit { needed: whole, offered: offered() });
    }
    let (mut placement, _) =
        split.unwrap_or_else(|| split_placement(model, &usable, objective)).map_err(|e| match e {
            Error::DoesNotFit { needed, .. } => Error::DoesNotFit { needed, offered: offered() },
            Error::NoPlan => Error::NoPlan,
        })?;
    left_out.append(&mut placement.left_out);
    placement.left_out = left_out;
    Ok(placement)
}

// The model split over `usable` in ring order, with standby replicas, and the time its fetches take spread over the
// first tokens.
fn split_placement<K: Clone>(
    model: &Model,
    usable: &[&Device<K>],
    objective: Objective,
) -> Result<(Placement<K>, f64), Error> {
    // a larger network is planned in a group: the fastest devices that hold the model together, at most GROUP of
    // them; the others stand by as replicas where a window fits them
    let all_usable = usable.to_vec();
    let (usable, solved) = if usable.len() > GROUP {
        let small = group(model, usable);
        match split(model, &small, objective) {
            // the windows did not divide among the fewest that hold the model: the whole group is tried
            Err(Error::DoesNotFit { .. }) => {
                let mut fastest = usable.to_vec();
                fastest.sort_by(|a, b| a.seconds_per_layer.total_cmp(&b.seconds_per_layer));
                fastest.truncate(GROUP);
                fastest.sort_by_key(|d| usable.iter().position(|u| std::ptr::eq(*u, *d)));
                let solved = split(model, &fastest, objective);
                (fastest, solved)
            }
            solved => (small, solved),
        }
    } else {
        (usable.to_vec(), split(model, usable, objective))
    };
    let (windows, gpu, head, seconds) = solved?;
    let mut stages = Vec::new();
    let mut start = 0;
    // the ring order from the head on
    let n = usable.len();
    for k in 0..n {
        let (d, w, g) = (usable[(head + k) % n], windows[(head + k) % n], gpu[(head + k) % n]);
        if w > 0 {
            stages.push((d, start..start + w, g));
            start += w;
        }
    }
    let fetch = stages
        .iter()
        .enumerate()
        .map(|(i, (d, layers, _))| {
            let ends = if i == 0 { model.head_bytes + model.tail_bytes } else { 0 };
            fetch_seconds(d, u64::from(layers.end - layers.start) * model.layer_bytes + ends)
        })
        .sum::<f64>()
        / AMORTIZED_TOKENS;
    let standby = replicas(model, &stages, &all_usable);
    // a device that neither computes nor stands by is not needed
    let left_out = all_usable
        .iter()
        .filter(|d| {
            !stages.iter().any(|(s, ..)| std::ptr::eq(*s, **d)) && !standby.iter().any(|(r, ..)| std::ptr::eq(*r, **d))
        })
        .map(|d| (d.id.clone(), LeftOut::NotNeeded))
        .collect();
    let placement = Placement {
        stages: stages
            .into_iter()
            .map(|(d, layers, gpu_layers)| Stage { node: d.id.clone(), layers, gpu_layers })
            .collect(),
        replicas: standby
            .into_iter()
            .map(|(d, layers, gpu_layers)| Replica { node: d.id.clone(), layers, gpu_layers })
            .collect(),
        left_out,
        seconds_per_token: seconds,
    };
    Ok((placement, fetch))
}

// The MILP: how many layers each device holds, in ring order.
//
//   w_i ∈ [0, L] integer   layers on device i
//   y_i ∈ {0, 1}           device i holds a window
//   f_i ∈ {0, 1}           device i is the head, holding the embedding and the output
//
//   Σ w_i = L;  y_i ≤ w_i ≤ L·y_i;  Σ f_i = 1;  f_i ≤ y_i
//   w_i·(layer + kv) + f_i·(head + tail) ≤ offered_i          (Q1: never more than offered)
//
// A device with a GPU lends it the last g_i layers of its window (NO-51); on the head the output goes along:
//
//   g_i ∈ [0, w_i] integer   layers on the GPU;  h_i ∈ {0, 1}: it uses the GPU, h_i ≤ g_i ≤ L·h_i
//   z_i = f_i ∧ h_i          the output on the GPU
//   g_i·(layer + kv) + z_i·tail ≤ gpu_i
//   (w_i − g_i)·(layer + kv) + f_i·(head + tail) − z_i·tail ≤ offered_i      (an integrated GPU frees nothing)
//
// The ring is a cycle: any device in use can be the head, and the layers run on from it around the ring order.
//
//   minimise Σ w_i·t_i + Σ g_i·(t_gpu_i − t_i) + Σ y_i·link_i: one token's compute plus every hop around the ring
// the names are the formulation's above
#[allow(clippy::cast_precision_loss, clippy::too_many_lines, clippy::many_single_char_names)]
fn split<K>(
    model: &Model,
    devices: &[&Device<K>],
    objective: Objective,
) -> Result<(Vec<u32>, Vec<u32>, usize, f64), Error> {
    let n = devices.len();
    let total = f64::from(model.layers);
    let per_layer = (model.layer_bytes + model.kv_bytes_per_layer) as f64;
    let ends = (model.head_bytes + model.tail_bytes) as f64;
    let mut pb = RowProblem::default();
    // latency counts every stage's time; throughput counts only the slowest stage's
    let latency = objective == Objective::Latency;
    let cost = |c: f64| if latency { c } else { 0.0 };
    // a layer the device must first fetch costs its fetch, spread over the first tokens, whatever the objective
    let fetch_layer = |d: &Device<K>| fetch_seconds(d, model.layer_bytes) / AMORTIZED_TOKENS;
    let w: Vec<_> = devices
        .iter()
        .map(|d| pb.add_integer_column(cost(d.seconds_per_layer) + fetch_layer(d), 0.0..=total))
        .collect();
    let y: Vec<_> = devices.iter().map(|d| pb.add_integer_column(cost(d.link_seconds), 0.0..=1.0)).collect();
    let fetch_ends = |d: &Device<K>| fetch_seconds(d, model.head_bytes + model.tail_bytes) / AMORTIZED_TOKENS;
    let f: Vec<_> = devices.iter().map(|d| pb.add_integer_column(fetch_ends(d), 0.0..=1.0)).collect();
    // a device without a GPU has its GPU columns fixed at zero; presolve takes them out
    let gain = |d: &Device<K>| d.gpu.map_or(0.0, |g| g.seconds_per_layer - d.seconds_per_layer);
    let top = |d: &Device<K>, v: f64| if d.gpu.is_some() { v } else { 0.0 };
    let g: Vec<_> = devices.iter().map(|d| pb.add_integer_column(cost(gain(d)), 0.0..=top(d, total))).collect();
    let h: Vec<_> = devices.iter().map(|d| pb.add_integer_column(0.0, 0.0..=top(d, 1.0))).collect();
    let z: Vec<_> = devices.iter().map(|d| pb.add_integer_column(0.0, 0.0..=top(d, 1.0))).collect();
    if !latency {
        let slowest = pb.add_column(1.0, 0.0..);
        for i in 0..n {
            pb.add_row(
                0.0..,
                [
                    (slowest, 1.0),
                    (w[i], -devices[i].seconds_per_layer),
                    (g[i], -gain(devices[i])),
                    (y[i], -devices[i].link_seconds),
                ],
            );
        }
    }
    let tail = model.tail_bytes as f64;

    pb.add_row(total..=total, w.iter().map(|&c| (c, 1.0)).collect::<Vec<_>>());
    pb.add_row(1.0..=1.0, f.iter().map(|&c| (c, 1.0)).collect::<Vec<_>>());
    for i in 0..n {
        pb.add_row(0.0.., [(w[i], 1.0), (y[i], -1.0)]);
        pb.add_row(..=0.0, [(w[i], 1.0), (y[i], -total)]);
        pb.add_row(..=0.0, [(f[i], 1.0), (y[i], -1.0)]);
        let ram = devices[i].offered_bytes as f64;
        match devices[i].gpu {
            None => pb.add_row(..=ram, [(w[i], per_layer), (f[i], ends)]),
            Some(gpu) => {
                pb.add_row(..=0.0, [(g[i], 1.0), (w[i], -1.0)]);
                pb.add_row(..=0.0, [(g[i], 1.0), (h[i], -total)]);
                pb.add_row(..=0.0, [(h[i], 1.0), (g[i], -1.0)]);
                pb.add_row(..=0.0, [(z[i], 1.0), (f[i], -1.0)]);
                pb.add_row(..=0.0, [(z[i], 1.0), (h[i], -1.0)]);
                pb.add_row(-1.0.., [(z[i], 1.0), (f[i], -1.0), (h[i], -1.0)]);
                pb.add_row(..=gpu.offered_bytes as f64, [(g[i], per_layer), (z[i], tail)]);
                if gpu.shares_memory {
                    pb.add_row(..=ram, [(w[i], per_layer), (f[i], ends)]);
                } else {
                    pb.add_row(..=ram, [(w[i], per_layer), (g[i], -per_layer), (f[i], ends), (z[i], -tail)]);
                }
            }
        }
    }

    let mut highs = pb.optimise(Sense::Minimise);
    highs.make_quiet();
    highs.set_option("time_limit", TIME_LIMIT.as_secs_f64());
    highs.set_option("threads", 1);
    let solved = highs.try_solve().map_err(|_| Error::NoPlan)?;
    match solved.status() {
        // a time limit still leaves the best plan found so far; the sum below tells whether there is one
        HighsModelStatus::Optimal | HighsModelStatus::ReachedTimeLimit => {}
        HighsModelStatus::Infeasible => return Err(Error::DoesNotFit { needed: model.whole_bytes(), offered: 0 }),
        _ => return Err(Error::NoPlan),
    }
    let columns = solved.get_solution().columns().to_vec();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let windows: Vec<u32> = columns.iter().take(n).map(|v| v.round() as u32).collect();
    if windows.len() != n || windows.iter().sum::<u32>() != model.layers {
        return Err(Error::NoPlan);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let gpu: Vec<u32> = (0..n).map(|i| (columns[3 * n + i].round() as u32).min(windows[i])).collect();
    let head = (0..n).max_by(|&a, &b| columns[2 * n + a].total_cmp(&columns[2 * n + b])).ok_or(Error::NoPlan)?;
    if windows[head] == 0 {
        return Err(Error::NoPlan);
    }
    // what one token takes around the ring, whichever objective chose the plan
    let seconds = (0..n)
        .filter(|&i| windows[i] > 0)
        .map(|i| window_seconds(devices[i], windows[i], gpu[i]) + devices[i].link_seconds)
        .sum();
    Ok((windows, gpu, head, seconds))
}

// A standby copy for every stage that some unused device can hold: each device stands by for at most one
// window, the largest window first, each taking the fastest spare device that can hold it.
fn replicas<'a, K>(
    model: &Model,
    stages: &[(&Device<K>, Range<u32>, u32)],
    usable: &[&'a Device<K>],
) -> Vec<(&'a Device<K>, Range<u32>, u32)> {
    let mut spare: Vec<&'a Device<K>> =
        usable.iter().copied().filter(|d| !stages.iter().any(|(s, ..)| std::ptr::eq(*s, *d))).collect();
    // a device that already holds the weights stands by first, then the quicker
    let missing = |d: &Device<K>| (1.0 - d.held.clamp(0.0, 1.0)) * d.fetch_seconds_per_byte;
    spare.sort_by(|a, b| missing(a).total_cmp(&missing(b)).then(capacity(model, a).1.total_cmp(&capacity(model, b).1)));
    let mut order: Vec<usize> = (0..stages.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(stages[i].1.len()));
    let mut replicas = Vec::new();
    for i in order {
        let layers = &stages[i].1;
        let width = layers.end - layers.start;
        if let Some((at, g)) =
            spare.iter().enumerate().find_map(|(at, d)| fit(model, d, width, i == 0).map(|g| (at, g)))
        {
            replicas.push((spare.remove(at), layers.clone(), g));
        }
    }
    replicas.sort_by_key(|(_, layers, _)| layers.start);
    replicas
}

/// The plan for every device of `placement` leaving, solved ahead so a departure is answered at once (NO-43).
/// A device whose departure leaves the model without room has no entry: its leaving stops the model.
#[must_use]
pub fn backups<K: Clone + PartialEq>(
    model: &Model,
    devices: &[Device<K>],
    placement: &Placement<K>,
) -> Vec<(K, Placement<K>)> {
    backups_for(model, devices, placement, Objective::Latency)
}

/// [`backups`] solved for `objective`, as the plan they stand in for was.
#[must_use]
pub fn backups_for<K: Clone + PartialEq>(
    model: &Model,
    devices: &[Device<K>],
    placement: &Placement<K>,
    objective: Objective,
) -> Vec<(K, Placement<K>)> {
    placement
        .stages
        .iter()
        .filter_map(|stage| {
            let rest: Vec<_> = devices.iter().filter(|d| d.id != stage.node).cloned().collect();
            place_for(model, &rest, objective).ok().map(|plan| (stage.node.clone(), plan))
        })
        .collect()
}

#[cfg(test)]
mod tests;
