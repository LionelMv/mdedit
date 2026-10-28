# Code blocks and inline code — specification

Code is where Markdown editors most often go wrong, and it is where WordCraft
currently goes wrong (see `wordcraft-markdown-findings.md`). This document is the
contract for mdedit. Each rule has an ID (`CB-n`) so tests and issues can refer to it.

Normative reference: CommonMark 0.31 §4.4 (indented code), §4.5 (fenced code),
§6.1 (code spans); GFM spec for the same sections.

---

## 1. Parsing (crate `syntax`)

| ID | Rule |
|----|------|
| CB-1 | A fence is ≥ 3 backticks or ≥ 3 tildes, indented ≤ 3 spaces. A backtick fence's info string may not contain a backtick. |
| CB-2 | The closing fence uses the **same character**, is **at least as long** as the opener, is indented ≤ 3 spaces, and holds nothing but optional trailing spaces. |
| CB-3 | An unclosed fence runs to the end of the enclosing container (document, list item, quote). Mark the block `closed = false` and show a subtle "unclosed" hint; don't swallow the rest of the doc silently in the UI. |
| CB-4 | The **info string is kept verbatim** (`CodeBlockInfo.info`). `lang` = first word, after backslash-escape and entity decoding, normalized through an alias table (`rs→rust`, `py→python`, `sh/bash/shell/zsh→bash`, `yml→yaml`, `js→javascript`, `ts→typescript`, `c++/cpp/cxx→cpp`, …). Unknown languages stay as-is (no highlight). |
| CB-5 | Content lines are **literal**: no inline parsing, no entity decoding, no backslash escapes, no smart quotes, no emoji shortcodes, no autolinks. |
| CB-6 | Strip from each content line up to *N* leading spaces where *N* = the opening fence's indentation (CommonMark §4.5). Everything else, **including tabs**, is kept byte-for-byte. Tabs are never converted to spaces in the buffer. |
| CB-7 | Blank lines inside the block are content. Trailing blank lines before the closing fence are content too (fenced code); for indented code, trailing blank lines are not part of the block. |
| CB-8 | Indented code: ≥ 4 columns of indentation (tab = advance to next multiple of 4) after a non-paragraph line. Can't interrupt a paragraph. |
| CB-9 | Code blocks nest inside list items and block quotes; fence indentation is measured relative to the container's content column. |
| CB-10 | A code block is **one block node** (`BlockKind::CodeBlock`), never one paragraph per line. |
| CB-11 | Inline code: a run of *n* backticks closes only at a run of exactly *n* backticks. Line endings become spaces; if the content both begins and ends with a space (and isn't all spaces), strip one from each side. No escapes inside. |

Since the buffer holds raw text (see architecture §1.3), rules CB-4/CB-6/CB-7
are mostly about **not destroying** data: the parser records ranges and the
renderer reads straight from the rope.

## 2. Display (crates `layout`, `render`, `highlight`)

| ID | Rule |
|----|------|
| CB-20 | The whole block draws as one **card**: background colour from theme, 1px border, rounded corners, padding (12 px h / 8 px v), full content width. |
| CB-21 | Font: the system `monospace` family from fontconfig (fallback chain: user setting → `monospace` → `DejaVu Sans Mono` → `Liberation Mono` → bundled font). Size = body size × 0.9 by default, configurable. |
| CB-22 | Tabs render to the next tab stop (default width 4, setting `code.tab_width`), computed in columns of the monospace font. Optional visible-whitespace mode. |
| CB-23 | Long lines: default **no wrap + horizontal scroll inside the card** (code shape is meaning). Setting `code.wrap = true` soft-wraps with a hanging indicator. Never wrap in the middle of the file's own layout. |
| CB-24 | Syntax highlighting with `syntect` by `lang`. Highlight state is cached per line so an edit re-highlights from the edited line until the parser state matches the cached state again. Large blocks (> 5 000 lines) highlight off-thread, viewport first. |
| CB-25 | Header row (on hover or when focused): language label (click → language selector with search, like MarkText's `codeBlockLanguageSelector`), copy button, optional line numbers toggle. |
| CB-26 | Spell-check, grammar check and auto-correct are **off** inside code blocks and inline code. |
| CB-27 | Live-preview: when the caret is outside the block, fences are hidden and only the card shows; when the caret is inside, the opening and closing fence lines become visible (dimmed), editable as text. |
| CB-28 | Special languages render a preview *below* the code (code stays editable): `mermaid` (if `mmdc` available), `math`/`latex`/`tex` blocks via the math renderer, `diff` gets +/- line backgrounds. Preview failure shows the error inline, never blocks typing. |
| CB-29 | Printing/PDF: a block shorter than a page never breaks across pages; longer blocks break between lines and repeat no header. |
| CB-30 | Inline code: monospace, subtle background pill, 2 px horizontal padding, never split by the inline formatting toolbar. Backticks shown only when the caret is inside the span. |

## 3. Editing (crate `editor`)

| ID | Behaviour |
|----|-----------|
| CB-40 | Typing ```` ``` ```` (or `~~~`) optionally + language at the start of an empty paragraph, then **Enter**, creates a block: inserts the closing fence and puts the caret on the empty content line. Same via the `/` quick-insert menu and `Ctrl+Shift+K`. |
| CB-41 | **Enter** inside code inserts `\n` + the current line's leading whitespace (auto-indent). No list continuation, no smart behaviour. |
| CB-42 | **Tab / Shift+Tab** inside code indent/outdent the selected lines (setting: insert `\t` or N spaces; default: match what the block already uses, else the setting). |
| CB-43 | **Enter** on an empty last line, pressed twice (or `Ctrl+Enter`) exits the block into a new paragraph below. **↓** on the last line moves out of the block. |
| CB-44 | **Backspace** at the start of an empty code block turns it back into an empty paragraph (removes both fences). |
| CB-45 | **Paste** inside code is always plain text, unmodified (no Markdown conversion, no HTML). Pasting HTML that contains `<pre><code class="language-x">` elsewhere creates a fenced block with `lang = x`. |
| CB-46 | Auto-pairing of brackets/quotes inside code is a setting (`code.auto_pair`, default on), separate from prose auto-pair. |
| CB-47 | Select-all (`Ctrl+A`) pressed once inside a code block selects the block's content; pressed again selects the document. |
| CB-48 | Changing the language in the selector rewrites only the info string in the rope. |
| CB-49 | If the user types a line of backticks inside a block that would close it, nothing magic happens — it's text, and the parser closes the block exactly as CommonMark would (live feedback shows it). |

## 4. Export (crate `export`)

| ID | Rule |
|----|------|
| CB-60 | **Save** writes the rope; there is no Markdown "export" step, so the fence character, length, info string, tabs and trailing whitespace are untouched. |
| CB-61 | When mdedit *generates* a fence (new block, paste from HTML), the fence is longer than the longest run of the same character in the content, minimum 3. |
| CB-62 | HTML export: `<pre><code class="language-{lang}">` with HTML-escaped content, plus syntect classed spans and a theme CSS block. |
| CB-63 | PDF export: monospace, highlighted, rules CB-23 (wrap in print by default since paper can't scroll) and CB-29. |

## 5. Test plan for code

- Run the CommonMark spec examples for sections 4.4, 4.5, 6.1 against the `syntax` crate
  (pulldown-cmark already passes them; we test *our* range mapping on top).
- Round-trip property test (`proptest`): random docs with random fences, tabs,
  CRLF, info strings → open → save → bytes identical.
- Snapshot tests (`insta`) of the block index for tricky inputs:

````text
```rust title="main.rs"
fn main() {
	println!("tab-indented");      <- tab must survive
}
```

~~~~
```
nested backtick fence is content
```
~~~~

  ```
  indented fence: two spaces stripped
    four spaces become two
  ```

- list item

  ```
  fenced code inside a list
  ```

```unclosed
runs to end of document
````

- Editing tests (CB-40…CB-49) as `egui_kittest` UI tests.
