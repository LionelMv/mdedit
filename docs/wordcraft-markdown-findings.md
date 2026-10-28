# WordCraft Markdown: how code blocks are handled, and what's wrong

Studied: `storytold/wordcraft` commit `d522bfd` (2026-10-27), version 0.11.0.
Files: `crates/formats/src/markdown/{blocks.rs,export.rs}`,
`crates/formats/src/model/{to_doc.rs,types.rs}`.

These findings come from reading the 0.11.0 source, **then testing with the installed
WordCraft Flatpak, first 0.7.0 and then 0.11.0** (`wordcraft-cli convert in.md out.md`,
see "Verified results" below). Both versions give byte-identical output on every case.

## Verified results (WordCraft 0.7.0 and 0.11.0, `wordcraft-cli convert x.md out.md`, 2026-10-28)

Same output from both versions; none of the problems are fixed in 0.11.0.

`^I` = TAB, `$` = end of line (`cat -A` output).

| Case | Input | Output | Problem |
|---|---|---|---|
| Language + tab | ```` ```make ```` / `all:` / `^Icc main.c` | ```` ``` ```` / `all:` / `    cc main.c` | P1 + P2 confirmed |
| Two blocks | ```` ```python ```` `print(1)` ```` ``` ```` *(blank)* ```` ```bash ```` `echo 2` ```` ``` ```` | **one** block: `print(1)` / `echo 2` | P3 merge confirmed; both languages gone |
| Code in list item | `- item` *(blank)* `  ```js` / `  let x = 1;` / `  ```` ` | `- item\` / `  let x = 1;` | **P7 (new)**: fences dropped, code becomes list text with a hard break |
| Tilde fence holding backticks | `~~~` / ```` ``` ```` / `nested` / ```` ``` ```` / `~~~` | ```` ```` ```` … ```` ```` ```` | OK (content kept, fence char changed) |
| Blank lines, trailing spaces | `line1`, 2 blank, `line4   ` | identical | OK |

## How it works

WordCraft imports Markdown into its word-processor model, not a Markdown tree:

1. `parse()` splits the file into lines and runs `expand_tabs()` on **every** line
   (`blocks.rs:144`).
2. A fence opener is recognized by `fence()` (`blocks.rs:232`), which returns only
   `(char, len)`.
3. Every line inside the fence becomes a **separate paragraph** with `Kind::Code`
   (`code_para`, `blocks.rs:382`; loop at `blocks.rs:453-465`).
4. Those paragraphs get the `Code` style: `Courier New` 10 pt, 0 spacing before and
   after (`to_doc.rs:31-41`, `types.rs:4`).
5. On export, runs of consecutive `Code` paragraphs are joined back into one fence
   (`export.rs:316-334`).

## Problems

### P1. The language / info string is thrown away (round-trip data loss)

`fence()` checks the info string (it rejects a backtick in it) but doesn't return it,
and `Para` has nothing to store it in. Export always writes a bare fence
(`out.push(fence.clone())`, `export.rs:329`).

````text
input                  after open → save in WordCraft
```rust                ```
fn main() {}           fn main() {}
```                    ```
````

Effect: syntax highlighting on GitHub/GitLab/static site generators is lost, and so
are attributes like `title=...` or `{.class}`. **Confirmed in 0.7.0 and 0.11.0.**

### P2. Leading tabs in code are turned into spaces

`expand_tabs()` (`blocks.rs:153-178`) changes leading tabs to spaces (tab stop 4)
on every line before block parsing, **including lines inside fenced code**. CommonMark
keeps code content literal (only the fence's own indentation is stripped).

Effect: a `Makefile` example in a fence breaks (make recipes need a TAB), and so do
Go code (gofmt uses tabs), TSV data and tab-indented YAML/Python samples, after one
open/save.

### P3. One paragraph per line, with no container

Because each line is its own paragraph:

- the code has no shared background/border "card", so it looks like ordinary
  text in Courier;
- page breaks can land anywhere inside a block (open upstream issue
  **#4131** reports this for Mermaid blocks);
- selecting, copying or deleting "the code block" as a unit isn't possible;
- the export has to guess where a block ends. Two code blocks that are only separated
  by a blank line (nothing else between them) come back as consecutive `Code`
  paragraphs with nothing to split them, so they are **merged into one block** on
  export (`export.rs:316-323`). **Confirmed in 0.7.0 and 0.11.0.**

### P7. Fenced code inside a list item loses its fences (found by testing)

A fence indented under a list item is exported as plain list-item text: the fences
disappear and the preceding line gets a `\` hard break. Seen in both 0.7.0 and
0.11.0.

### P4. No syntax highlighting, no language awareness

Even if P1 were fixed, nothing consumes the language: no highlighting, no
`mermaid`/math previews (requested in **#3800**, **#4131**).

### P5. Spell-check and grammar run inside code

Code paragraphs aren't excluded from proofing, so keywords and identifiers get
red/blue squiggles (reported in **#4131**, point 4).

### P6. Hard-coded `Courier New`

`MONO_FONT = "Courier New"` (`types.rs:4`). Most Linux distros don't ship it, so
the result depends on fontconfig substitution (often Liberation Mono, sometimes a
proportional fallback). Using the system's generic `monospace` family would be
more predictable on Linux.

### What WordCraft already gets right (worth copying)

- Closing fence rules: same char, ≥ length, ≤ 3 spaces indent, nothing after
  (`blocks.rs:459`).
- Generated fences are longer than any backtick run in the content (`export.rs:325-328`,
  fixed in #1788).
- Lines that would start a fence are escaped on export (#4119).
- Unclosed fence runs to end of input.
- Reference-definition-looking lines inside fences stay code (`blocks.rs:92-120`, tests).

## Related upstream issues (as of 2026-10-28)

| # | State | Title |
|---|-------|-------|
| 4131 | open | Markdown import: Support Mermaid diagram code blocks instead of raw text splitting across pages |
| 3800 | open | Add LaTeX and Mermaid Rendering for Markdown |
| 4130 | open | Markdown import: Parse inline and display LaTeX into native equations |
| 4464 | open | Paste Markdown from the clipboard as formatted text |
| 1805 | open | fix(formats): markdown table cells respect code spans and escapes |
| 1788 | closed | Markdown code fence outgrows any backtick run in its content |
| 4119 | closed | Markdown: escape lines that would start a fenced code block |

No open issue covers **P1** (info string dropped), **P2** (tabs expanded inside
fences), the **P3 merge**, or **P7**. These are the ones worth reporting.

## How mdedit avoids all of this

Mdedit never converts Markdown into another model: the rope *is* the file. See
`code-blocks.md` rules CB-4 (info string kept), CB-6 (tabs kept), CB-10 (one block
node), CB-20 (card), CB-24 (highlighting), CB-26 (no spell-check), CB-21 (system
monospace), CB-29 (no page split).

---

## Draft upstream issue (copy and paste if you want to report it)

> **Title:** Markdown round-trip drops the code fence language and turns tabs inside code into spaces
>
> **Version:** 0.11.0 (Flatpak, `wordcraft-cli convert t.md out.md`); also in 0.7.0
>
> **Steps**
> 1. Create `t.md`:
>    ````
>    ```make
>    all:
>    <TAB>cc main.c
>    ```
>    ````
> 2. Open it in WordCraft, then save it as Markdown.
>
> **Expected:** the file is unchanged: fence is ```` ```make ````, recipe line starts with a TAB.
>
> **Actual:** fence becomes ```` ``` ```` (the info string is gone) and the TAB becomes 4 spaces.
>
> **Cause**
> - `crates/formats/src/markdown/blocks.rs`: `fence()` doesn't return the info string,
>   `Para` has no field for it, and `export.rs` writes a bare fence.
> - `parse()` calls `expand_tabs()` on every line before block parsing, so lines
>   inside fenced code lose their leading tabs too. CommonMark §4.5 keeps code content
>   literal.
>
> **Suggested fix**
> - Keep the info string on the first `Code` paragraph (or a code-block container)
>   and write it back on export.
> - Expand tabs only where block structure is decided (indent measurement), not on
>   the stored text of fenced code lines.
> - Related: #4131 (code lines as separate paragraphs → page splits, spell-check in code).
>
> **Also seen:** two fenced blocks separated only by a blank line are saved as one
> block; a fenced block inside a list item loses its fences.
