# Unicode stress test

Caret movement, selection, deletion and word counting must treat each grapheme
cluster as one unit and never split a UTF-8 sequence.

## Emoji

Simple: 😀 🎉 🚀 ❤️ (with variation selector)

ZWJ sequences: 👨‍👩‍👧‍👦 family, 👩🏽‍💻 technologist with skin tone, 🏳️‍🌈 rainbow flag, 🧑‍🤝‍🧑 people holding hands

Skin tones: 👋🏻 👋🏼 👋🏽 👋🏾 👋🏿

Flags (regional indicators): 🇰🇪 🇯🇵 🇫🇷 🇧🇷 🇮🇱 🇸🇦

Keycaps: 1️⃣ 2️⃣ #️⃣ *️⃣

Tag sequence: 🏴󠁧󠁢󠁳󠁣󠁴󠁿 (Scotland)

## CJK

中文：快速的棕色狐狸跳过了懒狗。这是一个没有空格的长句子，用来测试换行是否在字符之间正确发生而不需要空格分隔。

日本語：素早い茶色の狐がのろまな犬を飛び越えた。カタカナ、ひらがな、漢字が混在しています。

한국어: 빠른 갈색 여우가 게으른 개를 뛰어넘었다. 한글 자모 조합: 한 = ㅎ + ㅏ + ㄴ.

Fullwidth: ＡＢＣ１２３！ and halfwidth katakana: ｶﾀｶﾅ

**粗体中文** and *斜体日本語* and `代码` and [链接](https://example.com)

## Right-to-left

Arabic: مرحبا بالعالم. هذه فقرة باللغة العربية لاختبار اتجاه النص من اليمين إلى اليسار.

Hebrew: שלום עולם. זוהי פסקה בעברית לבדיקת כיווניות.

Mixed: The word «مرحبا» means hello, and שלום means peace; numbers 123 stay LTR.

Arabic with diacritics: بِسْمِ اللَّهِ الرَّحْمَنِ الرَّحِيمِ

## Combining marks

Precomposed é (U+00E9) vs decomposed é (e + U+0301) — they must look identical.

Stacked: ạ̈̃ q̣̇ Z̴̡̛͎̟̼a̸̧̛̺l̷̢̙g̶̜̈́o̵̢͋ (Zalgo)

Devanagari: नमस्ते दुनिया — क्षत्रिय (conjuncts)

Thai: สวัสดีชาวโลก (no spaces between words)

Tamil: வணக்கம் உலகம்

## Other

Non-BMP math: 𝔘𝔫𝔦𝔠𝔬𝔡𝔢 𝕄𝕒𝕥𝕙 𝒮𝒸𝓇𝒾𝓅𝓉

Zero-width space between "a" and "b": a​b. Zero-width joiner: a‍b. Soft hyphen: super­cali­fragilistic.

Non-breaking space: 100 km. Narrow NBSP: 10 000.

Line separator (U+2028) next: here and paragraph separator (U+2029): there.

Box drawing: ┌─┬─┐ │ │ │ └─┴─┘

| Lang | Text |
|------|------|
| zh | 你好 |
| ar | مرحبا |
| emoji | 👨‍👩‍👧‍👦 |

```text
Code with unicode: λ → ∀x ∈ ℝ, 👍 中文 مرحبا
```

# 标题 with mixed 🎯 heading
