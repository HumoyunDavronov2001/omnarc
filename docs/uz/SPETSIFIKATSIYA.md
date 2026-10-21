# Omnarc — mahsulot spetsifikatsiyasi (v1.1, 13.10.2026)

> **Oʻzbekcha oʻqish nusxasi (13.10.2026, v1.1).** Asl hujjat ingliz tilida: [SPEC.md](../../SPEC.md).

> Omnarc 1.0 ning barcha ilovalari, ekranlari va funksiyalari — har biri o'zini quradigan bosqich va bugungi holati
> bilan. **Nima** quriladi va nima uchun — [TALABLAR.md](TALABLAR.md); **qanday** quriladi — [REJA.md](REJA.md);
> ekranlar qanday ko'rinishi — [design/concept.html](../../design/concept.html).

**Holat:** ✓ tayyor · ◐ qisman · ○ boshlanmagan. **Bosqich:** [REJA.md §11](REJA.md) dagi B0–B9 bosqichlari;
«B? → Bn» deb belgilangan bandning rejada hali bosqichi yo'q, Bn esa taklif (§16). **Talab:** band amalga
oshiradigan talab ([TALABLAR.md](TALABLAR.md) §3–§6; PLAN (REJA) bo'limlari «P§» ko'rinishida). **Dizayn:** ekranning
maketi dizayn konsepsiyasida bo'lsa — uning bo'lim raqami, bo'lmasa «—».

## 1. Sanoq

| Qism | Ekranlar | Funksiyalar | ✓ tayyor | ◐ qisman | ○ boshlanmagan | Tayyorlik |
|---|---|---|---|---|---|---|
| Dvigatel | 0 | 30 | 9 | 8 | 13 | 43 % |
| Tugun agenti va API | 0 | 42 | 16 | 14 | 12 | 55 % |
| Panel | 24 | 7 | 5 | 12 | 14 | 35 % |
| Ish stoli ilovasi | 4 | 5 | 0 | 0 | 9 | 0 % |
| Android ilovasi | 7 | 8 | 0 | 2 | 13 | 7 % |
| iOS ilovasi | 8 | 6 | 0 | 2 | 12 | 7 % |
| Brauzer tuguni | 3 | 2 | 0 | 0 | 5 | 0 % |
| Tashkilot boshqaruvi | 0 | 7 | 0 | 0 | 7 | 0 % |
| Ommaviy tarmoq va rele | 0 | 7 | 0 | 0 | 7 | 0 % |
| O'qitish | 0 | 4 | 0 | 0 | 4 | 0 % |
| Orollar | 0 | 3 | 0 | 0 | 3 | 0 % |
| O'lchovlar, saytlar va relizlar | 3 | 9 | 2 | 2 | 8 | 25 % |
| Dizayn tizimi va barcha ilovalardagi sifat | 1 | 13 | 6 | 4 | 4 | 57 % |
| **Jami** | **50** | **143** | **38** | **44** | **111** | **31 %** |

«Tayyorlik» ustunida qisman bajarilgan band yarim hisoblanadi. Bosqichlar bo'yicha — har bosqich nimani topshirishi
kerak:

| Bosqich | Bandlar | ✓ | ◐ | ○ |
|---|---|---|---|---|
| B0 | 10 | 9 | 1 | 0 |
| B1 | 16 | 12 | 2 | 2 |
| B2 | 28 | 7 | 20 | 1 |
| B3 | 16 | 10 | 5 | 1 |
| B4 | 18 | 0 | 10 | 8 |
| B5 | 53 | 0 | 5 | 48 |
| B6 | 23 | 0 | 0 | 23 |
| B7 | 9 | 0 | 0 | 9 |
| B8 | 13 | 0 | 1 | 12 |
| B9 | 7 | 0 | 0 | 7 |

## 2. Omnarc nimalardan iborat

| Qism | Odamlar nimani ko'radi | Qayerda ishlaydi | Qaysi bosqichda quriladi |
|---|---|---|---|
| Dvigatel (`libomnarc`, `omnarc-engine`, `omnarc-gate`) | hech narsa — u faqat hisoblaydi | har bir qurilma | B1, B4, B5, B7 |
| Tugun agenti (`omnarc-node`) | buyruq qatori; mahalliy portdagi panel | kompyuterlar, serverlar; ilovalar ichida | B1–B3 |
| API | ilovalar va agentlar uchun OpenAI, Claude Messages va Ollama API'lari | tugun agenti | B8 |
| Panel | klasterning veb interfeysi: Halqa va yana sakkizta bo'lim | brauzer, ish stoli ilovasi | B2–B8 |
| Ish stoli ilovasi | tray'dagi halqa, panelli oyna, o'rnatgichlar | Linux, Windows, macOS | B5 |
| Android ilovasi | «Kuchimni ulash», qoidalar, yordamchi, vidjetlar | Android 8+ | B5 |
| iOS ilovasi | «Kuchimni ulash», Tun rejimi, Live Activity, yordamchi | iOS 17+ | B5 |
| Brauzer tuguni | QR kod orqali klasterga qo'shiladigan brauzer oynasi | Chrome, Edge, Safari, Firefox | B5 |
| Tashkilot boshqaruvi (`omnarc-control`) | kirish, foydalanuvchilar, kalitlar, kvotalar, jadvallar, audit (ekranlari panelda) | server | B6 |
| Ommaviy tarmoq | katalog, isbotlar, kreditlar (ekranlari panelda) | rele va katalog serverlari | B6 |
| O'qitish | LoRA va RL ishlari (ekranlari panelda) | GPU tugunlari; RL javoblari uchun telefonlar va brauzerlar | B7 |
| Orollar | Thunderbolt 5 orqali ulangan Mac guruhlari, vLLM/SGLang'li GPU serverlar | Mac'lar, serverlar | B8 |
| O'lchovlar, saytlar, relizlar | natijalar sayti, hujjatlar sayti, ilova do'konlari | internet | B1, B9 |
| Dizayn tizimi | har bir ekran quriladigan tokenlar va komponentlar | barcha ilovalar | B0 → |

## 3. Dvigatel

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| EN-01 | llama.cpp'da qatlam oynalari [a, b) — kichik yamoqlar to'plami orqali; chiqish boshi n-qatlam hisoblanadi | funksiya | D1 | B1 | — | ✓ |
| EN-02 | C ABI `omnarc.h` v2: model ma'lumoti, bo'lakni ochish/yopish/ishga tushirish, KV'ni tashlash va qisqartirish, tokenizator, sampler | funksiya | D1 | B1 | — | ✓ |
| EN-03 | Yashirin holat oynaga kiradi va undan chiqadi; qurilmalar orasida f16, bf16 yoki f32 | funksiya | D2 | B1 | — | ✓ |
| EN-04 | Model xotiraga sig'masa, keyingi oynani diskdan oldindan o'qish | funksiya | D1 | B1 | — | ✓ |
| EN-05 | Dvigatel alohida jarayonda (`omnarc-engine`), agent bilan umumiy xotira orqali bog'lanadi | funksiya | P§4 | B1 | — | ✓ |
| EN-06 | Aniqlik darvozasi vositasi (`omnarc-gate`): KL, perplexity, top-1, to'liq moslik rejimi, istalgan bo'linish | funksiya | P§4 | B1 | — | ✓ |
| EN-07 | Backend'lar: CPU, Metal va Vulkan CI'da sinaladi; Windows'da CUDA va Vulkan yig'iladi | funksiya | W5 | B1 | — | ✓ |
| EN-08 | HIP/ROCm, OpenCL, SYCL va CANN backend'lari yig'iladi va darvozadan o'tkaziladi | funksiya | W5 | B5 | — | ○ |
| EN-09 | Qatlam oynalari ishlaydigan arxitekturalar: Llama, Qwen3 | funksiya | D1 | B1 | — | ✓ |
| EN-10 | Uchta qo'llanish uchun arxitekturalar: MoE oilalari (Qwen3.5, DeepSeek-V4, GLM-5, Kimi) va Gemma 4 | funksiya | D4 | B4 | — | ○ |
| EN-11 | Yamoqlar har hafta eng yangi llama.cpp ustiga qayta qo'yiladi; upstream'ga PR qilib yuboriladi | funksiya | P§4 | B1 | — | ◐ |
| EN-12 | Taxminiy dekodlash bilan uzluksiz pipeline (PipeSpec, FlowSpec, DiP-SD) | funksiya | D3, Y1 | B4 | — | ○ |
| EN-13 | Taxmin manbalari: MTP boshlari, EAGLE-3, kichik model, n-gram'lar; sekin qurilmalar uchun katta taxmin daraxtlari | funksiya | D3 | B4 | — | ○ |
| EN-14 | Issiq va sovuq neyronlar (PowerInfer) | funksiya | D4 | B4 | — | ○ |
| EN-15 | MoE ekspertlari birga faollashishiga qarab joylanadi, oldindan taxmin qilinadi va olib kelinadi; issiq ekspertlar nusxalanadi (EPLB) | funksiya | D4 | B4 | — | ○ |
| EN-16 | Sahifali KV kesh, radix prefiks kesh, q8/q4 KV | funksiya | P§4 | B4 | — | ○ |
| EN-17 | KV kesh qo'shni qurilmaga uzluksiz nusxalanadi, shuning uchun zaxira qayta hisoblamasdan davom etadi | funksiya | R3, Y7 | B3 | — | ○ |
| EN-18 | Ring attention: uzun kontekst ko'p qurilma xotirasiga taqsimlanadi | funksiya | Y9 | B4 | — | ○ |
| EN-19 | f8e4m3 faollashuvlar, har token uchun o'z masshtabi bilan | funksiya | T3 | B4 | — | ○ |
| EN-20 | Uzluksiz batching | funksiya | D6 | B4 | — | ○ |
| EN-21 | Prefill va dekod turli qurilmalarda | funksiya | D6 | B4 | — | ○ |
| EN-22 | Bitta model nusxasidan har qurilmaga o'z kvantlashi (kuchli qurilmaga Q8, kuchsiziga Q4) | funksiya | D6, Y5 | B4 | — | ○ |
| EN-23 | Aniqlik rejimi: modelning butun nusxalarida parallel yo'llar, ular orasida qisqa xulosalar, ulardan tuzilgan bitta javob | funksiya | D8, Y10 | B4 | — | ○ |
| EN-24 | Aniqlik darvozasidan o'tgan NPU amallari (Hexagon, Apple Neural Engine, Intel/AMD NPU) | funksiya | D5, Y3 | B5 | — | ○ |
| EN-25 | Telefon ilovalari ichidagi dvigatel (bitta jarayon ichida, Rust yadro orqali) | funksiya | Q2, Q3 | B5 | — | ○ |
| EN-26 | Brauzerdagi dvigatel: WASM'dagi llama.cpp, ggml-webgpu bilan | funksiya | Q4 | B5 | — | ○ |
| EN-27 | Barcha dvigatellar uchun bitta faollashuv protokoli: ggml, MLX, WebGPU, vLLM/SGLang adapterlari | funksiya | D2, Y5 | B8 | — | ◐ |
| EN-28 | Nutqni matnga va matnni nutqqa aylantirish | funksiya | D7 | B7 | — | ○ |
| EN-29 | Rasmlar (stable-diffusion.cpp) | funksiya | D7 | B7 | — | ○ |
| EN-30 | Embedding'lar | funksiya | D7 | B7 | — | ○ |

## 4. Tugun agenti va API

### 4.1 Dastur va xizmat

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| NO-01 | `omnarc-node info`: qurilma o'zi haqida e'lon qiladigan ma'lumot | funksiya | T1 | B0 | — | ✓ |
| NO-02 | `omnarc-node id`: tugun kaliti (Ed25519), birinchi ishlatilganda yaratiladi | funksiya | P§3 | B1 | — | ✓ |
| NO-03 | `omnarc-node ring`: qo'lda ulangan halqa — bosh tugun va ishchi tugunlar, bitta chiqish tokeniga ketadigan vaqt | funksiya | D1 | B1 | — | ✓ |
| NO-04 | Doimiy ishlaydigan xizmat (systemd, launchd, Windows xizmati): panelni mahalliy portda beradi | funksiya | T1 | B2 | — | ◐ |
| NO-05 | Buyruq qatoridan klaster yaratish va unga qo'shilish (chipta, kod) | funksiya | P§3 | B2 | — | ◐ |
| NO-06 | Linux, Windows, macOS, Android va iOS uchun bitta Rust dasturi | funksiya | T1 | B5 | — | ◐ |
| NO-07 | Serverlar uchun Docker tasviri va Helm chart | funksiya | P§6 | B9 | — | ○ |

### 4.2 Tarmoq

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| NO-10 | QUIC havolalari tugun kaliti orqali ulanadi (iroh); tenzor freymlari oqimlar orqali boradi; faqat kutilgan qo'shni qabul qilinadi | funksiya | T2 | B1 | — | ✓ |
| NO-11 | NAT teshish; teshib bo'lmasa — o'z rele serverimiz | funksiya | T2 | B2 | — | ◐ |
| NO-12 | Topish: LAN'da mDNS, internetda DNS/pkarr | funksiya | T2 | B2 | — | ◐ |
| NO-13 | Juftlash: ichida 10 daqiqalik sir bo'lgan QR chipta; ikkala ekranda to'rtta emoji bilan tasdiqlash | funksiya | P§3 | B2 | 02 | ✓ |
| NO-14 | Klaster kaliti va a'zolik tokenlari (Biscuit): tugun, foydalanuvchi va admin huquqlari | funksiya | P§3 | B2 | — | ◐ |
| NO-15 | Gossip orqali sekundiga bir marta telemetriya | funksiya | P§3 | B2 | — | ◐ |
| NO-16 | 3–5 barqaror tugunda Raft: reja versiyasi, joylashuv, kalitlar | funksiya | P§3 | B2 | — | ◐ |
| NO-17 | Har havolaning kechikishi va o'tkazuvchanligi jonli o'lchanadi | funksiya | T4 | B2 | — | ✓ |
| NO-18 | Barcha aloqa turlari (Wi-Fi, Ethernet, USB-C, Thunderbolt, Wi-Fi Direct), bir vaqtda bir nechtasi | funksiya | T3, Y6 | B5 | — | ○ |
| NO-19 | Brauzer tugunlariga WebRTC, signal rele orqali | funksiya | Q4 | B5 | — | ○ |
| NO-20 | `omnarc.v1` xabar sxemalari (qurilma, juftlash, telemetriya, reja, klaster xizmati) va tenzor freymi | funksiya | P§3 | B0 | — | ✓ |

### 4.3 Modellar

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| NO-30 | Model Hugging Face'dan bir marta yuklanadi, BLAKE3 bilan xeshlanadi va P2P tarqatiladi (iroh-blobs) | funksiya | T5 | B2 | — | ✓ |
| NO-31 | Har qurilma faqat o'z tenzorlarini bayt oraliqlari bo'yicha oladi va uzilishdan keyin davom ettiradi | funksiya | T5 | B2 | — | ✓ |
| NO-32 | Klaster ishlata oladigan modellar: hajmi, kvantlash turlari va litsenziyalari ko'rsatilgan katalog | funksiya | — | B8 | — | ○ |

### 4.4 Rejalashtiruvchi

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| NO-40 | Profiler: qurilma qo'shilganda hisoblash, xotira tezligi, disk va NPU'ning 20 soniyalik o'lchovi | funksiya | R1 | B3 | — | ○ |
| NO-41 | HiGHS bilan joylashtirish (64 qurilma uchun ≤ 1 s; kattaroq tarmoqlar guruhlarga bo'linadi) | funksiya | R1, W6 | B3 | 04 | ○ |
| NO-42 | Har so'rov uchun zaxiralar orasidan yo'l tanlash (Parallax) | funksiya | R1 | B3 | — | ○ |
| NO-43 | Model ishlab turganda qurilmalar qo'shiladi va chiqadi; oldindan hisoblangan zaxira reja 1 s ichida ishga tushadi | funksiya | R2, R3 | B3 | — | ○ |
| NO-44 | Hech qachon sekinlashtirmaydi: bitta qurilmaga sig'adigan model sekinroq havola orqali bo'linmaydi | funksiya | R6, W11 | B3 | — | ○ |
| NO-45 | Har so'rovning rejimi: tezlik, o'tkazuvchanlik, sig'im, aniqlik | funksiya | R4 | B3 | 01 | ○ |
| NO-46 | Tensor parallel guruhlari faqat ≤ 1 ms havolalarda | funksiya | R5 | B3 | — | ○ |
| NO-47 | Batareya, qizish, jadval va bo'sh turish chegaralari ishni zaxiraga o'tkazadi | funksiya | Q5 | B5 | 03 | ○ |
| NO-48 | Reja takliflari: qatlamlarni qayerga ko'chirsa, halqa tezlashadi | funksiya | R1 | B3 | 01 | ○ |
| NO-49 | Qayta rejalash va KV nusxalari TLA+ da modellanadi va TLC'da tekshiriladi | funksiya | P§8 | B3 | — | ○ |
| NO-50 | Tarmoq simulyatsiyasida 1000 ta tasodifiy uzilish va qo'shilish ssenariysi — birorta javob yo'qolmaydi | funksiya | P§8 | B2 | — | ◐ |

### 4.5 API

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| AP-01 | Panel va ilovalar uchun boshqaruv API (Connect RPC), jonli oqim bilan | funksiya | P§3 | B2 | — | ◐ |
| AP-02 | OpenAI'ga mos: chat completions, completions, embeddings, models, oqimli javob | funksiya | A1 | B8 | — | ○ |
| AP-03 | Claude Messages'ga mos | funksiya | A1 | B8 | — | ○ |
| AP-04 | Ollama'ga mos | funksiya | A1 | B8 | — | ○ |
| AP-05 | Agentlar uchun backend: uzun takrorlanadigan ko'rsatmalar prefiks keshda saqlanadi; asbob chaqiruvlari, tuzilgan javob, uzun kontekstlar | funksiya | A4 | B8 | — | ○ |
| AP-06 | Model, kvota va muddat bo'yicha cheklangan API kalitlari (Biscuit) | funksiya | P§7 | B6 | — | ○ |
| AP-07 | Audio va rasm API'lari | funksiya | D7 | B7 | — | ○ |
| AP-08 | Metrikalar (Prometheus) va izlar (OpenTelemetry); sukut bo'yicha hech qayerga hech narsa yuborilmaydi | funksiya | A3 | B4 | — | ○ |

## 5. Panel

Panel — klasterning veb interfeysi. Uni har bir tugun beradi; ish stoli ilovasi uni o'z oynasida ko'rsatadi. Uning
to'qqizta bo'limi yon menyuda turadi: Halqa, Qurilmalar, Modellar, Suhbat, Kuzatuv, O'qitish, Tarmoq, Tashkilot,
Sozlamalar.

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| PA-01 | Ramka: bo'limlar yon menyusi, yuqori qator, pastki qator | ekran | U1 | B2 | 01 | ◐ |
| PA-02 | Birinchi ishga tushirish | ekran | U6 | B2 | — | ◐ |
| PA-03 | Halqa (bosh ekran) | ekran | U1 | B2 | 01 | ◐ |
| PA-04 | Qurilma qo'shish | ekran | U6 | B2 | 02 | ◐ |
| PA-05 | Qurilmalar | ekran | — | B2 | 01 | ◐ |
| PA-06 | Bitta qurilma | ekran | Q1, Q5 | B2 | — | ◐ |
| PA-07 | Odamlar (oila halqasi) | ekran | K1, Q6 | B6 | — | ○ |
| PA-08 | Modellar: katalog | ekran | — | B8 | — | ○ |
| PA-09 | «Sig'adimi?» va joylashuv rejasi | ekran | — | B3 | 04 | ○ |
| PA-10 | Ishlayotgan modellar va yuklashlar | ekran | T5 | B3 | — | ○ |
| PA-11 | Ilovalar va agentlar orqali foydalanish | ekran | A4 | B8 | — | ○ |
| PA-12 | Suhbat | ekran | D8 | B4 | — | ○ |
| PA-13 | Kuzatuv: token yo'li | ekran | — | B4 | 05 | ○ |
| PA-14 | Kuzatuv: havolalar va izlar | ekran | T4 | B4 | 05 | ○ |
| PA-15 | O'qitish | ekran | O1, O2 | B7 | — | ○ |
| PA-16 | Tarmoq: ommaviy tarmoq | ekran | A2, K3 | B6 | 06 | ○ |
| PA-17 | Kredit | ekran | K3 | B6 | 06 | ○ |
| PA-18 | Tashkilot: kirish | ekran | K2 | B6 | — | ○ |
| PA-19 | Tashkilot: foydalanuvchilar va rollar | ekran | K2 | B6 | — | ○ |
| PA-20 | Tashkilot: kalitlar va kvotalar | ekran | A3 | B6 | — | ○ |
| PA-21 | Tashkilot: jadvallar va siyosat | ekran | K2 | B6 | — | ○ |
| PA-22 | Tashkilot: audit va foydalanish | ekran | K2, A3 | B6 | — | ○ |
| PA-23 | Sozlamalar | ekran | U9 | B2 | — | ◐ |
| PA-24 | Buyruqlar paneli (⌘K) | ekran | U7 | B8 | 01 | ○ |
| PA-30 | Bildirishnomalar: qurilma chiqdi va ishni uning zaxirasi oldi, qurilma qo'shildi, model tayyor, xatolar | funksiya | P§9 | B3 | — | ○ |
| PA-31 | «Qo'llash» va «Rad etish» tugmalari bilan reja taklifi kartasi | funksiya | R1 | B3 | 01 | ○ |
| PA-32 | Hamma uchun halqa: jadval ko'rinishi, yoydan yoyga klaviatura fokusi, ovoz bilan o'qiladigan xulosa | funksiya | U4 | B2 | — | ✓ |
| PA-33 | Bir nechta model — ichma-ich halqalar; 128 tadan ortiq qurilmada yoylar guruhlanadi | funksiya | U1 | B4 | — | ○ |
| PA-34 | Paneldagi tirik belgi: kirish sahnasi, og'ish, qurilma qo'shilganda yangi yoy | funksiya | U11 | B2 | 01 | ✓ |
| PA-35 | Connect oqimi orqali jonli ma'lumot | funksiya | P§9 | B2 | — | ✓ |
| PA-36 | Har amal uchun klaviatura yorlig'i | funksiya | U7 | B8 | — | ○ |

**PA-01 Ramka.** *Ko'rsatadi:* logotip, belgilari bilan to'qqizta bo'lim, klaster nomi va sog'liq belgisi
(«6 qurilma · hammasi sog'lom»), qidiruv, «Qurilma qo'shish» tugmasi; pastda — ommaviy tarmoqni yoqish-o'chirish
tugmasi va kreditlar. *Amallar:* bo'limlar orasida o'tish; ⌘K ni ochish; qurilma qo'shish. *Holatlar:* Tashkilot
bo'limi faqat tashkilot klasterlarida chiqadi.

**PA-02 Birinchi ishga tushirish.** *Ko'rsatadi:* topilgan qurilma (nomi, backend, xotira), 20 soniyalik o'lchov,
LAN'da allaqachon bor klasterlar, ulush surgichi, yangi yoy o'sib chiqadigan tirik belgi. *Amallar:* klaster yaratish
yoki mavjudiga qo'shilish; ulushni belgilash. *Holatlar:* WebGPU yoki backend topilmasa → faqat CPU, bu ochiq
aytiladi; boshlanishdan yoy paydo bo'lguncha ≤ 60 s.

**PA-03 Halqa.** *Ko'rsatadi:* har bir qurilma — yoy (uzunligi — qatlamlar ulushi, rangi — yuklama); halqa bo'ylab
o'lchangan tezlikda yuradigan tokenlar, xira uchqunlar ko'rinishidagi taxmin tokenlari; markazda — token/s, model va
rejim; tashqi tomonda zaryad, harorat va batareya uchun ingichka belgilar; taxmin tokenlari qabul qilinishi, faol
so'rovlar, xotira va energiya kartalari; bandlik vaqti va harorati bilan qurilmalar ro'yxati; oxirgi 120 ms dagi
token yo'li; reja taklifi. *Amallar:* rejimni almashtirish (Tezlik, O'tkazuvchanlik, Sig'im, Aniqlik); model tanlash;
qurilmani ochish; jadval ko'rinishiga o'tish. *Holatlar:* hech qanday model ishlamayapti → «Model tanlang»; bitta
qurilma → bitta to'liq doira; qurilma chiqib ketsa → uning yoyi so'nadi va zaxirasi yonadi.

**PA-04 Qurilma qo'shish.** *Ko'rsatadi:* kompyuterda QR kod va 10 daqiqalik taymerli kod; ikkala ekranda bir xil
to'rtta emoji; oxirida yangi yoy va u nimani olgani («6 qatlam oldi · Vulkan · 8 GB · USB-C orqali 0,4 ms»).
*Amallar:* kodni skanerlash yoki kiritish; «Ha, mos keladi» yoki «Yo'q, farq qiladi» deb tasdiqlash. *Holatlar:* kod
muddati o'tgan; belgilar farq qiladi → qurilma rad etiladi; hammasi bo'lib ≤ 30 s.

**PA-05 Qurilmalar.** *Ko'rsatadi:* qurilmalar jadvali — nomi, turi, backend, xotira ulushi, qatlamlar, harorat,
batareya, qo'shni qurilma bilan havola, holat. *Amallar:* saralash, filtrlash, qurilmani ochish, qurilmani olib
tashlash. *Holatlar:* oflayn qurilmalar kulrang bo'ladi va oxirgi marta qachon ko'ringani yoziladi.

**PA-06 Bitta qurilma.** *Ko'rsatadi:* backend va hisoblash bloklari, xotira, harorat, batareya, qatlamlari,
qo'shnilari bilan havolalar (kechikishi va o'tkazuvchanligi bilan), o'lchov natijasi. *Amallar:* ulushni (xotira %,
GPU %, CPU yadrolari), jadvalni va «faqat bo'sh turganda» rejimini belgilash; o'lchovni qayta o'tkazish; olib tashlash.
*Holatlar:* batareya yoki qizish chegarasiga yetganda banner qurilma ishi zaxiraga o'tganini aytadi.

**PA-07 Odamlar.** *Ko'rsatadi:* oila yoki jamoa a'zolari — har biri o'z qurilmalari bilan, hamda yordamchidan faqat
ovoz orqali foydalanadigan telefonlar. *Amallar:* a'zoni QR kod bilan taklif qilish; a'zoni olib tashlash; kim qurilma
qo'sha olishini belgilash. *Holatlar:* har a'zoning suhbatlari o'z qurilmasida qoladi; panel buni aytib turadi.

**PA-08 Modellar: katalog.** *Ko'rsatadi:* modellar — hajmi, kvantlashi, litsenziyasi va klasterga sig'ish-sig'masligi;
Hugging Face'da qidiruv. *Amallar:* hajm, vazifa (matn, nutq, rasm, embedding) va litsenziya bo'yicha filtrlash;
«Sig'adimi?» ni ochish.

**PA-09 «Sig'adimi?»** *Ko'rsatadi:* modelga kerakli xotira klaster xotirasi bilan taqqoslanadi; hech narsa
yuklanmasdan oldin joylashuv rejasi halqada chizib ko'rsatiladi; qurilmalar o'lchovidan kelib chiqqan kutilayotgan
tezlik oralig'i. *Amallar:* ishga tushirish; boshqa kvantlashni tanlash. *Holatlar:* sig'maydi → qancha xotira
yetishmayotgani va yetishmovchilikni qaysi qurilma qoplashi.

**PA-10 Ishlayotgan modellar va yuklashlar.** *Ko'rsatadi:* har bir ishlayotgan model — halqasi, rejimi va yuklamasi
bilan; yuklashlar — har qurilma bo'lagi qancha yuklangani bilan. *Amallar:* to'xtatish, qayta ishga tushirish,
rejimni almashtirish; yuklashni to'xtatib turish.

**PA-11 Ilovalar va agentlar orqali foydalanish.** *Ko'rsatadi:* klasterning API manzili, kalitlar, hamda OpenClaw,
OpenCode va boshqa agentlar, OpenAI, Claude va Ollama SDK'lari uchun nusxa olishga tayyor sozlamalar. *Amallar:*
model, kvota va muddat bilan cheklangan kalit yaratish; kalitni bekor qilish; sinov chaqiruvini yuborish. *Holatlar:*
har kalit bo'yicha so'rovlar va sarflangan tokenlar.

**PA-12 Suhbat.** *Ko'rsatadi:* klaster bilan suhbat; har javob ostida uni hisoblagan qurilmalar, har biri o'z
rangida; aniqlik rejimida — nechta qurilma rozi bo'lgani. *Amallar:* model va rejimni tanlash; to'xtatish; nusxa
olish; ovoz bilan so'rash. *Holatlar:* model hali yuklanmoqda → javob jarayon chizig'i bilan kutib turadi.

**PA-13 Kuzatuv: token yo'li.** *Ko'rsatadi:* oxirgi soniyalarning vaqt chizig'i — har qurilmada hisob, tarmoq, taxmin
va bo'sh turish vaqti, oddiy pipeline bilan yonma-yon; bo'sh turish ulushi. *Amallar:* to'xtatib turish,
kattalashtirish, so'rovni tanlash. *Holatlar:* eng sekin bosqich belgilanadi.

**PA-14 Kuzatuv: havolalar va izlar.** *Ko'rsatadi:* qurilmalar orasidagi havolalar kechikishi va o'tkazuvchanligi
matritsasi; alohida so'rovlarning izlari. *Amallar:* havola yoki so'rovni ochish; izni eksport qilish.

**PA-15 O'qitish.** *Ko'rsatadi:* LoRA va RL ishlari, ularning raundlari, loss egri chizig'i, qatnashayotgan
tugunlar. *Amallar:* ish boshlash (asosiy model, ma'lumotlar, raundlar), to'xtatib turish, to'xtatish, adapterni
yuklab olish. *Holatlar:* faqat PyTorch'li GPU tugunlari o'qitadi; telefonlar va brauzerlar RL uchun javob yaratadi.

**PA-16 Tarmoq.** *Ko'rsatadi:* ommaviy tugunlar globusi va klasterlar orasidagi oqimlar; tugunlar, mamlakatlar va
klasterlar soni; qoidalar (xom matn qurilmangizda qoladi, har javob isbotlanadi, kreditlar ochiq jurnalda).
*Amallar:* ommaviy tarmoqqa qo'shilish yoki undan chiqish; qurilma nima berishini tanlash.

**PA-17 Kredit.** *Ko'rsatadi:* balans, shu oy ishlab topilgani, imzolangan kvitansiyalar va har birining ochiq
jurnaldagi yozuvi. *Amallar:* kvitansiyani tekshirish; kreditlarni ommaviy modellarga sarflash.

**PA-18 Tashkilot: kirish.** *Ko'rsatadi:* tashkilotning OIDC yoki LDAP/Active Directory tizimi orqali kirish.
*Holatlar:* roli mos kelmasa → foydalanuvchi nima qila olishi va kimga murojaat qilishi kerakligi.

**PA-19 Tashkilot: foydalanuvchilar va rollar.** *Ko'rsatadi:* foydalanuvchilar va ularning rollari (admin,
foydalanuvchi, tugun egasi). *Amallar:* taklif qilish, rolni o'zgartirish, olib tashlash.

**PA-20 Tashkilot: kalitlar va kvotalar.** *Ko'rsatadi:* har foydalanuvchi, jamoa va model bo'yicha API kalitlari va
kvotalar. *Amallar:* yaratish, cheklash, bekor qilish.

**PA-21 Tashkilot: jadvallar va siyosat.** *Ko'rsatadi:* idora kompyuterlari klaster uchun qachon ishlashi (masalan,
tunda) va nimani bera olishi. *Amallar:* kompyuter guruhlari uchun ish vaqti va chegaralarni belgilash; modellarga
ruxsat berish.

**PA-22 Tashkilot: audit va foydalanish.** *Ko'rsatadi:* audit jurnali va har foydalanuvchi, jamoa va model bo'yicha
sarflangan tokenlar. *Amallar:* filtrlash va eksport qilish.

**PA-23 Sozlamalar.** *Ko'rsatadi:* til (ingliz tili va yettita tarjima), mavzu (tun, kun), xavfsizlik (shu tugunning
kaliti, ishonchli qurilmalar), yangilanishlar, hech qayerga hech narsa yuborilmasligi haqidagi eslatma. *Amallar:*
ularni o'zgartirish.

**PA-24 Buyruqlar paneli.** *Ko'rsatadi:* panelning har bir amali — o'z klaviatura yorlig'i bilan. *Amallar:* yozib
topish va bajarish.

## 6. Ish stoli ilovasi

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| DE-01 | Tray: qurilma yuklamasiga qarab to'ladigan kichik halqa va uning menyusi | ekran | Q1, U8 | B5 | 03 | ○ |
| DE-02 | Oyna: ilova ichidagi panel | ekran | — | B5 | 01 | ○ |
| DE-03 | Ilovada birinchi ishga tushirish | ekran | U6 | B5 | — | ○ |
| DE-04 | Ulush va jadval | ekran | Q1 | B5 | — | ○ |
| DE-10 | Agent ilova ichida ishlaydi; xotira model uchun bo'sh qoladi (Tauri, ~10 MB) | funksiya | P§2 | B5 | — | ○ |
| DE-11 | Tizimga kirganda o'zi ishga tushadi va fonda ishlashda davom etadi | funksiya | Q1 | B5 | — | ○ |
| DE-12 | Imzolangan avtomatik yangilanishlar | funksiya | P§2 | B5 | — | ○ |
| DE-13 | Tizim bildirishnomalari | funksiya | — | B5 | — | ○ |
| DE-14 | O'rnatgichlar: MSI, DMG (notarizatsiya bilan), AppImage, deb, rpm, Flatpak, Homebrew, winget, Nix | funksiya | P§6 | B9 | — | ○ |

**DE-01 Tray.** *Ko'rsatadi:* yuklamaga qarab to'ladigan halqa belgisi. *Amallar (menyu):* Omnarc'ni ochish; ulashni
to'xtatib turish yoki davom ettirish; ulush darajasini tez tanlash; chiqish.

**DE-03 Birinchi ishga tushirish.** Paneldagi birinchi ishga tushirish bilan bir xil qadamlar va qo'shimcha ravishda
«tizimga kirganda ishga tushish».

**DE-04 Ulush va jadval.** *Ko'rsatadi:* xotira %, GPU % va CPU yadrolari uchun surgichlar; ish vaqti; «faqat bo'sh
turganda». *Amallar:* ularni o'zgartirish; chegaralar doim ko'rinib turadi.

## 7. Android ilovasi

Ilovada to'rtta bo'lim bor — Hissa, Klaster, Suhbat, Sozlamalar — dizayn konsepsiyasidagi telefondagidek.

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| AN-01 | Birinchi ishga tushirish | ekran | U6 | B5 | — | ○ |
| AN-02 | Hissa: «Kuchimni ulash» | ekran | P§9 | B5 | 03 | ○ |
| AN-03 | Qo'shilish: QR kodni skanerlash, belgilarni tasdiqlash | ekran | P§3 | B5 | 02 | ○ |
| AN-04 | Qoidalar: qachon ishlaydi | ekran | Q5 | B5 | 03 | ○ |
| AN-05 | Klaster: telefondagi halqa | ekran | U1 | B5 | — | ○ |
| AN-06 | Suhbat va yordamchi: matn, kuchsiz telefonlar uchun ovoz | ekran | Q6 | B5 | — | ○ |
| AN-07 | Sozlamalar | ekran | U9 | B5 | — | ○ |
| AN-10 | Zaryadlanayotganda ishlaydigan fon xizmati: API ≥ 34 da `specialUse`, undan pastda `dataSync` | funksiya | Q2 | B5 | — | ○ |
| AN-11 | Quick Settings plitkasi | funksiya | U8 | B5 | — | ○ |
| AN-12 | Glance vidjet | funksiya | U8 | B5 | — | ○ |
| AN-13 | Bildirishnomada jonli holat | funksiya | U8 | B5 | — | ○ |
| AN-14 | Vulkan, OpenCL va Hexagon NPU | funksiya | Q2 | B5 | — | ○ |
| AN-15 | Ilovadagi Rust yadro: CI'da Android uchun kompilyatsiya qilinadi; Kotlin'ga UniFFI bog'lamalari yozilmagan | funksiya | T1 | B5 | — | ◐ |
| AN-16 | Android 8 dan 16 gacha (API 26–36); Maestro oqimlari | funksiya | W4 | B5 | — | ○ |
| AN-17 | Umumiy tokenlar bilan Material 3 Expressive | funksiya | U2 | B5 | — | ◐ |

**AN-01 Birinchi ishga tushirish.** *Ko'rsatadi:* ilova nima qilishi, bildirishnoma va kamera ruxsatlari.
*Amallar:* klasterga qo'shilish (skanerlash) yoki telefondan faqat yordamchi sifatida foydalanish.

**AN-02 Hissa.** *Ko'rsatadi:* «Kuchingiz klasterda», klaster, telefondagi qatlamlar va model; zaryad, Wi-Fi va
harorat; bugungi tokenlar, energiya va kreditlar; bitta katta tugma — «Kuchimni ulash». *Amallar:* ulashni boshlash
yoki to'xtatish. *Holatlar:* o'z qoidalari bajarilishini kutmoqda (masalan, zaryadda emas) → qaysi qoida
bajarilmagani.

**AN-03 Qo'shilish.** *Ko'rsatadi:* kamera, keyin topilgan klaster («Uy klasteri · 5 qurilma»), keyin to'rtta emoji.
*Amallar:* qo'shilish; belgilar mos kelgan yoki kelmaganini tasdiqlash.

**AN-04 Qoidalar.** *Ko'rsatadi:* faqat zaryadda (tavsiya etiladi), faqat Wi-Fi'da, faqat ekran o'chiq bo'lganda,
batareya kamida 40 %, harorat chegarasi 40 °C, xotira ulushi, jadval. *Amallar:* har bir qoidani o'zgartirish.
*Holatlar:* chegaraga yetganda telefon ishini zaxira qurilmaga beradi — ekranda shu yoziladi.

**AN-05 Klaster.** *Ko'rsatadi:* shu telefonning yoyi ajratib ko'rsatilgan halqa, model, tezlik.

**AN-06 Suhbat va yordamchi.** *Ko'rsatadi:* suhbat; bosib turib gapiriladigan tugma; javob ovoz bilan qaytadi.
Xotirasi ≤ 4 GB bo'lgan telefon faqat shu mijoz bo'lib ishlaydi va hech narsa hisoblamaydi (Q6). *Amallar:* gapirish,
yozish, to'xtatish.

**AN-07 Sozlamalar.** *Ko'rsatadi:* til, mavzu, klaster va uning kaliti, ilova haqida.

## 8. iOS ilovasi

Android'dagi to'rtta bo'limning o'zi; iOS fonda uzoq ishlashga ruxsat bermaydi, shuning uchun telefon Tun rejimida
hissa qo'shadi.

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| IO-01 | Birinchi ishga tushirish | ekran | U6 | B5 | — | ○ |
| IO-02 | Hissa: «Kuchimni ulash» | ekran | P§9 | B5 | 03 | ○ |
| IO-03 | Qo'shilish: QR kodni skanerlash, belgilarni tasdiqlash | ekran | P§3 | B5 | 02 | ○ |
| IO-04 | Qoidalar: qachon ishlaydi | ekran | Q5 | B5 | 03 | ○ |
| IO-05 | Tun rejimi: zaryadlanayotganda soat va halqani ko'rsatadigan deyarli qora ekran | ekran | Q3 | B5 | 03 | ○ |
| IO-06 | Klaster: telefondagi halqa | ekran | U1 | B5 | — | ○ |
| IO-07 | Suhbat va yordamchi: matn, kuchsiz telefonlar uchun ovoz | ekran | Q6 | B5 | — | ○ |
| IO-08 | Sozlamalar | ekran | U9 | B5 | — | ○ |
| IO-10 | Qulf ekranida va Dynamic Island'da Live Activity | funksiya | U8 | B5 | 03 | ○ |
| IO-11 | Vidjet (WidgetKit) | funksiya | U8 | B5 | — | ○ |
| IO-12 | Siri va Shortcuts: «Kuchimni ulash» | funksiya | U8 | B5 | — | ○ |
| IO-13 | Metal va Neural Engine | funksiya | Q3 | B5 | — | ○ |
| IO-14 | Ilovadagi Rust yadro: CI'da iOS uchun kompilyatsiya qilinadi; Swift'ga UniFFI bog'lamalari bilan XCFramework yig'ilmagan | funksiya | T1 | B5 | — | ◐ |
| IO-15 | Umumiy tokenlar bilan Liquid Glass (iOS 26) | funksiya | U2 | B5 | — | ◐ |

**IO-05 Tun rejimi.** *Ko'rsatadi:* soat, sana, halqa, «Hissa qo'shmoqda · 4 qatlam · 31 °C». *Amallar:* oddiy
rejimga o'tish uchun ekranga teginish. *Holatlar:* zaryaddan uzilsa → Tun rejimi tugaydi va ish zaxiraga o'tadi.

**IO-10 Live Activity.** *Ko'rsatadi:* qulf ekranida tun bo'yi qilingan ish («41 300 token · 6 soat 12 daqiqa») va
Dynamic Island'da halqa.

iOS'ning qolgan ekranlari Android'dagilar bilan bir xil (§7).

## 9. Brauzer tuguni

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| BR-01 | QR koddan ochiladigan qo'shilish sahifasi | ekran | Q4, W8 | B5 | — | ○ |
| BR-02 | Hissa qo'shish | ekran | Q4 | B5 | — | ○ |
| BR-03 | Internet yo'q va sahifa keshlanmagan | ekran | Q4 | B5 | — | ○ |
| BR-10 | Loyihaning HTTPS manzilidan o'rnatiladigan, keshda qoladigan veb-ilova (PWA); sahifa ≤ 60 KB | funksiya | U10 | B5 | — | ○ |
| BR-11 | LAN tugunlariga WebRTC; signal rele orqali | funksiya | Q4 | B5 | — | ○ |

**BR-01 Qo'shilish sahifasi.** *Ko'rsatadi:* bu oynani qaysi klaster taklif qilayotgani, brauzerda WebGPU bor-yo'qligi,
oyna qancha xotira bera olishi. *Amallar:* qo'shilish; skanerlashdan boshlab ≤ 30 s. *Holatlar:* WebGPU yo'q → oyna
baribir CPU'da RL javoblari uchun qo'shila oladi yoki unga yordam bera olmasligi aytiladi.

**BR-02 Hissa qo'shish.** *Ko'rsatadi:* oynaning halqadagi yoyi, uning token/s tezligi, «bu oynani yopmang».
*Amallar:* to'xtatish.

**BR-03 Internet yo'q.** *Ko'rsatadi:* brauzer tuguni uchun sahifa oldin bir marta keshlangan bo'lishi kerakligi;
uning o'rniga ilovani qanday o'rnatish.

## 10. Tashkilot boshqaruvi

Ekranlar panelda (PA-18 dan PA-22 gacha); bu yerda — ularning ortidagi xizmat.

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| OR-01 | GPUStack serveri asosida qurilgan `omnarc-control` xizmati (Python, FastAPI, PostgreSQL 17) | funksiya | K2 | B6 | — | ○ |
| OR-02 | OIDC va LDAP/Active Directory orqali kirish | funksiya | K2 | B6 | — | ○ |
| OR-03 | Rollar va ruxsatlar (RBAC) | funksiya | K2 | B6 | — | ○ |
| OR-04 | Kvotalar va token hisobi | funksiya | A3 | B6 | — | ○ |
| OR-05 | Kompyuter guruhlari uchun ish vaqti jadvallari | funksiya | K2 | B6 | — | ○ |
| OR-06 | Audit jurnali | funksiya | K2 | B6 | — | ○ |
| OR-07 | Markaziy siyosat: qurilmalar nima bera olishi, qaysi modellarga ruxsat berilgani | funksiya | K2 | B6 | — | ○ |

## 11. Ommaviy tarmoq va rele

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| PU-01 | `omnarc-relay`: rele, WebRTC signal, katalog | funksiya | T2 | B2 | — | ○ |
| PU-02 | Ommaviy tugunlar katalogi; DHT orqali topish | funksiya | K3, T2 | B6 | 06 | ○ |
| PU-03 | Birinchi va oxirgi qatlamlar doim so'rov egasining qurilmasida bajariladi | funksiya | K3 | B6 | 06 | ○ |
| PU-04 | Har javob bilan TOPLOC isboti | funksiya | K3 | B6 | 06 | ○ |
| PU-05 | Ishning 1–2 % tasodifiy tugunda qayta hisoblanadi; reyting; aldovchi tugunlar chiqariladi | funksiya | K3 | B6 | — | ○ |
| PU-06 | Kreditlar: hisoblash hisobga olinadi, kvitansiyalar Ed25519 bilan imzolanadi va shaffoflik jurnaliga yoziladi | funksiya | K3 | B6 | 06 | ○ |
| PU-07 | Konteynerlarda haqiqiy protokol bilan 1000+ tugun va kamida 20 ta real qurilma | funksiya | W7 | B6 | — | ○ |

## 12. O'qitish

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| TR-01 | GPU tugunlarida internet orqali DiLoCo bilan LoRA fine-tuning (PyTorch) | funksiya | O1 | B7 | — | ○ |
| TR-02 | RL rollout'lar: telefonlar va brauzerlar javob yaratadi, o'qituvchi tugun o'rganadi | funksiya | O2 | B7 | — | ○ |
| TR-03 | Internet orqali 4+ tugunda kam resursli til uchun LoRA (B7 darvozasi) | funksiya | O1 | B7 | — | ○ |
| TR-04 | Nazorat nuqtalari bilan ishlar — tugunlar chiqib ketsa ham davom etadi | funksiya | O1 | B7 | — | ○ |

## 13. Orollar

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| IS-01 | MLX oroli: Thunderbolt 5 RDMA orqali birlashtirilgan Mac'lar (exo'dan) | funksiya | Y5 | B8 | — | ○ |
| IS-02 | GPU oroli: vLLM yoki SGLang bilan NVIDIA va AMD serverlari (GPUStack'dan) | funksiya | Y5 | B8 | — | ○ |
| IS-03 | NVIDIA serverda o'tkazuvchanlik vLLM'nikidan kam emas | funksiya | W9 | B8 | — | ○ |

## 14. O'lchovlar, saytlar va relizlar

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| BE-01 | Olti platformada CI, dvigatel backend'lari, yamoqlarni har hafta yangi llama.cpp ustiga qayta qo'yish | funksiya | P§8 | B0 | — | ✓ |
| BE-02 | B1 darvozasi vositalari: nazorat yig'indisi bilan model yuklash, aniqlik darvozasi skripti, solishtirish uchun yig'ilgan prima.cpp | funksiya | W10 | B1 | — | ✓ |
| BE-03 | B1 darvozasi: Llama 3.x 8B, Qwen3 32B va R1-Distill 70B da aniqlik | funksiya | P§11 | B1 | — | ◐ |
| BE-04 | B1 darvozasi: 70B model Linux va Windows kompyuterlarida birgalikda ishlaydi | funksiya | P§11 | B1 | — | ○ |
| BE-05 | B1 darvozasi: bitta chiqish tokeniga ketadigan vaqt prima.cpp'nikidan yomon emas | funksiya | P§11 | B1 | — | ○ |
| BE-06 | `omnarc-bench`: W1–W13 bo'yicha exo, prima.cpp, Parallax, distributed-llama, llama.cpp RPC, Mesh LLM, colibri bilan solishtirish | funksiya | W1–W13 | B9 | — | ○ |
| BE-07 | Freym tahlilchisi, protobuf va GGUF o'quvchisi uchun 24 soatlik fuzzing | funksiya | P§8 | B2 | — | ◐ |
| BE-08 | Natijalar sayti | ekran | W10 | B9 | — | ○ |
| BE-09 | Hujjatlar sayti (Astro Starlight, 8 til, oflayn qidiruv) | ekran | U9 | B9 | — | ○ |
| BE-10 | O'z HTTPS domenidagi loyiha sayti, brauzer tuguni sahifasi bilan | ekran | Q4 | B5 | — | ○ |
| BE-11 | Imzolangan relizlar: cosign imzolari, SBOM, SLSA kelib chiqish dalili | funksiya | P§2 | B9 | — | ○ |
| BE-12 | Do'konlar: Google Play, F-Droid, App Store va TestFlight, Homebrew, winget, Flathub, Docker, Helm | funksiya | P§12 | B9 | — | ○ |

## 15. Dizayn tizimi va barcha ilovalardagi sifat

| ID | Nima | Turi | Talab | Bosqich | Dizayn | Holat |
|---|---|---|---|---|---|---|
| DS-01 | Tokenlar bitta manbadan CSS, Swift, Kotlin va TypeScript'ga; kontrast darvozasi | funksiya | U2 | B0 | 07 | ✓ |
| DS-02 | O'z serverimizdan beriladigan shriftlar: Inter va IBM Plex Mono | funksiya | U3 | B0 | 07 | ✓ |
| DS-03 | Noto Sans SC, Arabic va Devanagari — tilga qarab yuklanadi | funksiya | U3 | B5 | — | ○ |
| DS-04 | Qurilgan va sinalgan komponentlar (unit, vizual, axe): Button, Card, Chip, Field, Icon, Kbd, Logo, Meter, Ring, Status, Switch, LivingRing | funksiya | U2, U4 | B0 | 07 | ✓ |
| DS-05 | Ekranlarga hali kerak bo'lgan komponentlar: Tabs, Table, Dialog, Toast, Menu, Select, Slider, Tooltip, Progress, Empty state, Skeleton, QR code, Emoji check, Code block, Message, Voice button, Agreement badge | funksiya | U2 | B2–B8 | — | ◐ |
| DS-06 | Grafiklar: uPlot ma'lumot qatorlari, token yo'li vaqt chizig'i, havolalar matritsasi | funksiya | U4 | B4 | 05 | ○ |
| DS-07 | Globus: MapLibre GL, deck.gl va oflayn Protomaps xarita plitkalari | funksiya | A2 | B6 | 06 | ○ |
| DS-08 | Tirik belgi (WebGL2) va WebGL2 bo'lmasa chiqadigan SVG varianti | funksiya | U11 | B0 | 01 | ✓ |
| DS-09 | Dizayn konsepsiyasi sahifasi va README rasmlari | ekran | — | B0 | — | ✓ |
| XC-01 | Xitoy, ispan, hind, arab (o'ngdan chapga), fransuz, portugal va rus tillariga tarjimalar | funksiya | U9 | B5 | — | ○ |
| XC-02 | WCAG 2.2 AA, faqat klaviatura bilan boshqarish, kam harakat rejimi, rang ko'rligiga chidamli grafiklar | funksiya | U4, U5 | B0 → | 07 | ◐ |
| XC-03 | Byudjetlar: panel JS ≤ 180 KB, brauzer sahifasi ≤ 60 KB, 1000 qurilmada 60 fps, LCP ≤ 1,5 s, sovuq start ≤ 1 s | funksiya | U10 | B2 → | — | ◐ |
| XC-04 | Har havolada shifrlash; modellar BLAKE3 bilan tekshiriladi | funksiya | P§7 | B2 | — | ◐ |
| XC-05 | Sukut bo'yicha hech qayerga hech narsa yuborilmaydi | funksiya | P§2 | B0 → | — | ✓ |

## 16. Reja hali bosqichga joylamagan bandlar

Hozir yo'q. Reja hali joylamagan band o'z jadvalida «B? → Bn» deb belgilanadi va kelishilguncha shu yerda turadi.
Birinchi sanoq (13.10.2026) oltita shunday band topdi; o'sha kuni ular rejaga qo'shildi (REJA.md v3.2, §11):

| ID | Band | Bosqich | Nega aynan u yerda |
|---|---|---|---|
| PA-12 | Suhbat | B4 | javoblarda ularni hisoblagan qurilmalar va aniqlik rejimi ko'rinadigan birinchi bosqich |
| PA-23 | Sozlamalar | B2 | birinchi panelga til va mavzu kerak |
| PA-30 | Bildirishnomalar | B3 | B3 dan boshlab qurilmalar ish paytida qo'shiladi va chiqadi |
| BE-07 | 24 soatlik fuzzing | B2 | freym tahlilchisi va protobuf B2 da tarmoqqa chiqadi |
| DS-03 | Noto shriftlari | B5 | ilovalar tarjimalari bilan chiqadi |
| XC-01 | Tarjimalar | B5 | ilovalar tarjimalari bilan chiqadi; hujjatlar esa B9 da |

## 17. Uchta qo'llanish — har birini nima ta'minlaydi

| Qo'llanish | Dvigatel | Agent va API | Ekranlar | Nima isbotlaydi |
|---|---|---|---|---|
| O'z modelingizda agentlar | EN-10, EN-15, EN-16, EN-21 | NO-30, NO-31, NO-41, NO-44, AP-02–AP-05 | PA-09, PA-11 | W13 |
| Oila halqasi | EN-25, EN-28 | NO-13, NO-14, NO-47 | PA-07, AN-06, IO-07 | B7 darvozasi |
| Aniqlik rejimi | EN-23 | NO-45 | PA-03, PA-12 | W12 |
