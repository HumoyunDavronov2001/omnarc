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
}

/// One device's window of layers, in ring order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage<K> {
    pub node: K,
    pub layers: Range<u32>,
}

/// A standby copy of a stage's window on another device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replica<K> {
    pub node: K,
    pub layers: Range<u32>,
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

/// The time `HiGHS` may take for one plan (W6: a plan for 64 devices within a second).
pub const TIME_LIMIT: Duration = Duration::from_millis(900);

/// Places `model` on `devices`, which stand in ring order.
///
/// # Errors
///
/// [`Error::DoesNotFit`] when no assignment stays within what the devices offer.
pub fn place<K: Clone>(model: &Model, devices: &[Device<K>]) -> Result<Placement<K>, Error> {
    let one_layer = model.layer_bytes + model.kv_bytes_per_layer;
    let (usable, small): (Vec<_>, Vec<_>) = devices.iter().partition(|d| d.offered_bytes >= one_layer);
    let mut left_out: Vec<_> = small.into_iter().map(|d| (d.id.clone(), LeftOut::TooSmall)).collect();
    let offered = || devices.iter().map(|d| d.offered_bytes).sum();

    // R6: a model that fits on one device runs there, on the fastest that can hold it, and nowhere split
    let whole = model.whole_bytes();
    let fastest = usable
        .iter()
        .filter(|d| d.offered_bytes >= whole)
        .min_by(|a, b| a.seconds_per_layer.total_cmp(&b.seconds_per_layer));
    if let Some(best) = fastest {
        let layers = 0..model.layers;
        let mut replicas = Vec::new();
        for d in &usable {
            if std::ptr::eq(*d, *best) {
                continue;
            }
            if d.offered_bytes >= whole {
                replicas.push(Replica { node: d.id.clone(), layers: layers.clone() });
            } else {
                left_out.push((d.id.clone(), LeftOut::NotNeeded));
            }
        }
        return Ok(Placement {
            stages: vec![Stage { node: best.id.clone(), layers }],
            replicas,
            left_out,
            seconds_per_token: f64::from(model.layers) * best.seconds_per_layer,
        });
    }
    if usable.iter().map(|d| d.offered_bytes).sum::<u64>() < whole {
        return Err(Error::DoesNotFit { needed: whole, offered: offered() });
    }

    let (windows, head, seconds) = split(model, &usable).map_err(|e| match e {
        Error::DoesNotFit { needed, .. } => Error::DoesNotFit { needed, offered: offered() },
        Error::NoPlan => Error::NoPlan,
    })?;
    let mut stages = Vec::new();
    let mut start = 0;
    // the ring order from the head on
    let n = usable.len();
    for (d, &w) in (0..n).map(|k| (usable[(head + k) % n], &windows[(head + k) % n])) {
        if w == 0 {
            left_out.push((d.id.clone(), LeftOut::NotNeeded));
        } else {
            stages.push((d, start..start + w));
            start += w;
        }
    }
    let replicas = replicas(model, &stages, &usable);
    Ok(Placement {
        stages: stages.into_iter().map(|(d, layers)| Stage { node: d.id.clone(), layers }).collect(),
        replicas,
        left_out,
        seconds_per_token: seconds,
    })
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
// The ring is a cycle: any device in use can be the head, and the layers run on from it around the ring order.
//
//   minimise Σ w_i·t_i + Σ y_i·link_i: one token's compute plus every hop around the ring
#[allow(clippy::cast_precision_loss)]
fn split<K>(model: &Model, devices: &[&Device<K>]) -> Result<(Vec<u32>, usize, f64), Error> {
    let n = devices.len();
    let total = f64::from(model.layers);
    let per_layer = (model.layer_bytes + model.kv_bytes_per_layer) as f64;
    let ends = (model.head_bytes + model.tail_bytes) as f64;
    let mut pb = RowProblem::default();
    let w: Vec<_> = devices.iter().map(|d| pb.add_integer_column(d.seconds_per_layer, 0.0..=total)).collect();
    let y: Vec<_> = devices.iter().map(|d| pb.add_integer_column(d.link_seconds, 0.0..=1.0)).collect();
    let f: Vec<_> = (0..n).map(|_| pb.add_integer_column(0.0, 0.0..=1.0)).collect();

    pb.add_row(total..=total, w.iter().map(|&c| (c, 1.0)).collect::<Vec<_>>());
    pb.add_row(1.0..=1.0, f.iter().map(|&c| (c, 1.0)).collect::<Vec<_>>());
    for i in 0..n {
        pb.add_row(0.0.., [(w[i], 1.0), (y[i], -1.0)]);
        pb.add_row(..=0.0, [(w[i], 1.0), (y[i], -total)]);
        pb.add_row(..=0.0, [(f[i], 1.0), (y[i], -1.0)]);
        pb.add_row(..=devices[i].offered_bytes as f64, [(w[i], per_layer), (f[i], ends)]);
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
    let head = (0..n).max_by(|&a, &b| columns[2 * n + a].total_cmp(&columns[2 * n + b])).ok_or(Error::NoPlan)?;
    if windows[head] == 0 {
        return Err(Error::NoPlan);
    }
    Ok((windows, head, solved.objective_value()))
}

// A standby copy for every stage that some unused device can hold: each device stands by for at most one
// window, the largest window first, each taking the fastest spare device that can hold it.
fn replicas<K: Clone>(model: &Model, stages: &[(&Device<K>, Range<u32>)], usable: &[&Device<K>]) -> Vec<Replica<K>> {
    let mut spare: Vec<&Device<K>> =
        usable.iter().copied().filter(|d| !stages.iter().any(|(s, _)| std::ptr::eq(*s, *d))).collect();
    spare.sort_by(|a, b| a.seconds_per_layer.total_cmp(&b.seconds_per_layer));
    let mut order: Vec<usize> = (0..stages.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(stages[i].1.len()));
    let mut replicas = Vec::new();
    for i in order {
        let layers = &stages[i].1;
        let need = model.window_bytes(layers.end - layers.start, i == 0);
        if let Some(at) = spare.iter().position(|d| d.offered_bytes >= need) {
            let d = spare.remove(at);
            replicas.push(Replica { node: d.id.clone(), layers: layers.clone() });
        }
    }
    replicas.sort_by_key(|r| r.layers.start);
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
    placement
        .stages
        .iter()
        .filter_map(|stage| {
            let rest: Vec<_> = devices.iter().filter(|d| d.id != stage.node).cloned().collect();
            place(model, &rest).ok().map(|plan| (stage.node.clone(), plan))
        })
        .collect()
}

#[cfg(test)]
mod tests;
