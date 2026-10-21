# Omnarc — reja (v4, 22-oktabr 2026) — o‘qish nusxasi

Asosiy hujjat inglizcha: [PLAN.md](../../PLAN.md). Eski v3.2: [docs/history/REJA-v3.2.md](../history/REJA-v3.2.md).

## 0. Nega reja o‘zgardi

21-oktabrda bitta oddiy Windows kompyuter oddiy llama.cpp bilan Llama 3.1 8B da **8,2 tok/s** berdi. Shu kompyuter
va noutbuk Omnarc’ning o‘z halqasida Qwen3-14B bilan **0,37 tok/s** berdi, birinchi token 122 soniyada keldi
([bench/reality](../../bench/reality/README.md)). O‘n kun eng qiyin qismlarni birinchi qurdik: o‘z engine yamog‘i,
halqa, jonli almashinuv, MILP rejalashtiruvchi, TLA+. Birortasi ham oddiy asos bilan solishtirilmadi.

Besh saboq:

1. **Bitta qurilmaga sig‘adigan modelni bo‘lish faqat sekinlashtiradi.** Bizning halqa ham, Mesh LLM sinovi ham
   (68 → 21 → 12 tok/s) shuni ko‘rsatdi.
2. **Tezlik — fizika.** Har token har faol og‘irlikni bir marta o‘qiydi: tezlik ≈ bir tokenda o‘qiladigan bayt ÷
   xotira tezligi. Qurilma qo‘shish sig‘im va bir vaqtdagi xizmatni oshiradi, bitta javob tezligini emas. Oddiy
   kompyuterlarda katta modelni tez qiladigan yagona narsa — MoE modellar (har tokenda og‘irlikning kichik qismi o‘qiladi).
3. **Formula yolg‘on, o‘lchov haqiqat.** GPU formulasi 11 qatlam dedi — 5 tasi sig‘di. Tezlik 0,35 s deyildi — 2,69 s chiqdi.
4. **Boshqalar qurgani oldinda.** llama.cpp o‘zi sig‘diradi (`-fit`), MoE ekspertlarini CPU’ga qo‘yadi (`-ncmoe`),
   so‘rovlarni birlashtiradi, promptni keshlaydi, OpenAI API va veb-chat beradi, har OS va GPU uchun tayyor binar
   chiqaradi. Mesh LLM (Apache-2.0, v0.75, Rust, iroh) modelni macOS, Linux va Windows kompyuterlarga bo‘ladi,
   har qurilmaga faqat o‘z GGUF bo‘laklarini beradi, NAT ortidan relay orqali ulanadi, taklif tokenli yopiq mesh quradi.
5. **Foydalanuvchi arxitekturani emas, mahsulotni ko‘radi.** Xatolar hech kimga ko‘rinmadi — har qurilma jurnalida
   qoldi. Yangi Windows’ga o‘rnatish kunlab kross-yig‘ish bo‘ldi.

## 1. Qoidalar

- **R-1 Qayta yozmaymiz, olamiz.** Har qism uchun eng yaxshi ochiq loyiha o‘zicha ishlatiladi. Omnarc faqat
  ulovchi qismni va hech qayerda yo‘q narsani yozadi; yaxshilanishni avval asl loyihaga yuboradi.
- **R-2 O‘lchovsiz da’vo yo‘q.** Har raqam [bench/](../../bench/) dagi fayldan: apparat, versiya, buyruq, natija.
  Maqsadlar o‘lchovdan keyin qo‘yiladi.
- **R-3 Asos.** Har funksiya o‘sha qurilmalarda oddiy llama.cpp bilan solishtiriladi; sekinroq yoki ishonchsizroq
  bo‘lsa chiqarilmaydi.
- **R-4 Avval oltin yo‘l.** O‘rnatish → qo‘shilish → model tanlash → savol. Yangi odam terminalsiz o‘tmaguncha
  boshqa narsa qurilmaydi.
- **R-5 Xato — mahsulot qismi.** Har qurilmadagi har xato panelda oddiy so‘z bilan ko‘rinadi.

## 2. Omnarc nima

> **Sizning kompyuterlaringiz birortasi yolg‘iz ko‘tara olmaydigan modelni ishlatadi va hammaga bir vaqtda
> xizmat qiladi — maxfiy, internetsiz, bulut hisobisiz.**

Va’da: sig‘im, bir vaqtda ko‘p odam, maxfiylik va oflayn, model ishga tushishidan oldin ko‘rsatiladigan **o‘lchangan** tezlik.

Hech qachon va’da qilinmaydi: eng kuchli qurilmaning o‘zidan tez javob; ma’lumot markazi tezligi; sig‘adigan modelni
bo‘lish; o‘lchanmagan har qanday raqam.

## 3. Qurmaymiz — ustiga quramiz

| Ehtiyoj | Nimadan olinadi | Omnarc nima qo‘shadi |
|---|---|---|
| Har OS va GPU’da hisoblash | **llama.cpp** rasmiy relizlari, `llama-server` | hech narsa — o‘zicha |
| Modelni bir necha kompyuterga bo‘lish | **Mesh LLM** (nomzod, S0 da tekshiriladi) | S0 dan keyin yetishmayotgani — asl loyihaga hissa sifatida |
| Mesh LLM rad etilsa | **llama.cpp RPC**, faqat shifrlangan tunnel ichida (o‘zida parol yo‘q) | tunnel va joylashtirish |
| Modellar | Hugging Face; qurilmalar bo‘laklarni mesh orqali ulashadi | — |
| Chat | Mesh LLM yoki llama-server veb-chati | — |
| Telefon | o‘sha veb-chat telefon brauzerida, mahalliy tarmoqda | v4 da telefon — mijoz, hisoblovchi emas |
| O‘rnatuvchi va yangilash | tanlangan ish muhitining o‘z paketi; kerak bo‘lsa Tauri 2 yangilovchisi | har OS uchun bitta o‘rnatuvchi |

Omnarc o‘ziniki: har qurilma egasining budjeti va soatlari, yangi odam tushunadigan panel, tashkilot funksiyalari,
«sig‘adimi va qanchalik tez?» savoliga o‘lchangan javob. Bularning har biri avval asl loyihaga taklif qilinadi.

### 3.1 O‘z kodimiz nima bo‘ladi

Hammasi git tarixida qoladi; S0 o‘rnini nima egallashini hal qilmaguncha hech narsa o‘chirilmaydi.

| Qism | v4 |
|---|---|
| Engine yamog‘i, libomnarc ABI 1–7, halqa, jonli almashinuv, KV nusxa | muzlatildi; S0 tanlagan ish muhiti egallaydi |
| HiGHS MILP, profiler formulalari | muzlatildi; joylash ish muhitiniki, o‘lcham `-fit` dan, tezlik o‘lchanadi |
| f8, n-gram, TLA+ | muzlatildi; llama.cpp’ning o‘z spekulyativ dekodlashi |
| iroh, serverdagi relay, P2P model | faqat ish muhiti o‘zi qilmasa qoladi (Mesh LLM ham iroh ishlatadi) |
| Egasining budjeti va soatlari (Q1, Q7) | qoladi: ish muhitiga taklif qilinadi; rad etilsa — o‘zimizniki |
| Dizayn tizimi, tirik halqa | Omnarc’ning o‘z yuzlari uchun qoladi |
| Android ilova | faqat mijoz, yoki brauzer bilan almashadi |

## 4. Bosqichlar

Bosqich darvozasi real qurilmalarda o‘lchanib `bench/` ga yozilganda tugaydi. Oldingisi o‘tmaguncha keyingisi boshlanmaydi.

| Bosqich | Ish | Darvoza |
|---|---|---|
| **S0 Tekshirish** | Mesh LLM va oddiy llama.cpp’ni bor qurilmalarga rasmiy binardan o‘rnatish, bir xil modellarni ikkalasida o‘lchash | yig‘ishsiz o‘rnatiladi; NAT ortida yopiq mesh quriladi; sig‘adigan model butun ishlaydi, oddiy llama.cpp’dan 10 % ichida; sig‘maydigani bo‘linadi, tezligi yoziladi; gpt-oss-20b va Qwen3-30B-A3B ikki qurilmada yoziladi; qurilma chiqib ketishi yoziladi. Natija: `bench/s0/` va ish muhiti tanlovi |
| **S1 Oltin yo‘l** | Har OS uchun bitta o‘rnatuvchi, QR taklif, panel: qurilmalar, nima beradi, nima qayerda, har xato so‘z bilan. Har relizdan keyin real klasterda avtomatik savol | yangi odam ikki kompyuterga o‘rnatib, terminalsiz ≤ 10 daqiqada javob oladi (Omnarc’ni ko‘rmagan odam bilan o‘lchanadi) |
| **S2 Hovuz** | Modelni butun sig‘diradigan har qurilmada nusxa; so‘rov eng bo‘sh nusxaga; har qurilmada batching; egasining budjeti va soatlari | N odam bir vaqtda, har biri bitta foydalanuvchi tezligining kamida yarmida; 2 va 4 qurilmada o‘lchanadi |
| **S3 Katta MoE** | Hech bir qurilmaga sig‘maydigan model, ish muhiti bo‘ladi, o‘lchangan tezlik oldindan ko‘rsatiladi | katta MoE (hajmi S0 dan) oddiy kompyuterlarda S0 o‘lchagan tezlikda, agent (OpenCode yoki OpenClaw) bilan belgilangan vazifalarda |
| **S4 Tashkilot** | hisoblar va rollar, API kalitlar, kvota, jadval («tungi ofis»), audit | 5 kishilik jamoa bir hafta bitta klasterda ishlaydi |

S4 dan keyin har yangi g‘oya o‘lchangan sabab bilan kiradi: hisoblovchi telefon, brauzer, o‘qitish, ochiq tarmoq.

## 5. Kim to‘laydi, qachon

v3.2 §14 pozitsiyasi qoladi: hisoblash foydalanuvchiniki, u sotilmaydi; atrofidagi narsa sotiladi, daromad tashkilotdan.

| Tarif | Nima beradi | Tayyor |
|---|---|---|
| Bepul | o‘z qurilmalaringiz, hech narsa chiqmaydi, oflayn | S1 |
| Uy | bir nechta qurilma birga | S2 |
| Tashkilot | hisoblar, kvota, jadval, audit | S4 |
| Dasturchi | API kalitlar, model katalogi, foydalanish | S4 |

## 6. Qanday ishlaymiz

- Repoda bir vaqtda bitta sessiya; tugaganda xotirada topshirish yozuvi.
- Real klasterda avtomatik sinov: har joylashdan keyin kichik modelga savol; yiqilsa ish to‘xtaydi.
- Avval o‘lchov: bir soatlik tajriba bir kunlik koddan oldin.
- Reliz binarlari CI yoki bitta skriptli buyruqdan; qo‘lda kross-yig‘ish yo‘q.

## 7. Egasi hal qiladigan savollar

1. S0 dan keyin: asos Mesh LLM, yoki llama.cpp RPC + o‘zimizning yupqa boshqaruvchi.
2. Asl loyihaga hissa qo‘shamizmi yoki fork saqlaymizmi.
3. Serverning roli: faqat relay, yoki kattaroq budjetli hisoblovchi.
4. CI relizlarni yig‘ishi uchun yopiq branch’ga push.
5. S0 uchun qurilmalar: noutbuk (5 GB) bo‘linadigan modelni ko‘tarolmaydi, server boshqa qit’adagi umumiy VPS,
   Windows chiqib ketdi. Bitta mahalliy tarmoqda bo‘lish uchun u yerda ikkinchi kompyuter kerak.

[SPEC.md](../../SPEC.md) hali v3.2 bandlarini v3.2 bosqichlari bo‘yicha sanaydi; S0 ish muhitini tanlagach S0–S4 ga moslanadi.
