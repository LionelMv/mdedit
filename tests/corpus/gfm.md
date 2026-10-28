# GitHub Flavored Markdown

## Tables

| Left | Center | Right | Default |
|:-----|:------:|------:|---------|
| a    | b      | c     | d       |
| `code` | **bold** | *em* | [link](https://example.com) |
| pipe \| escaped | | empty cell | |

No leading/trailing pipes:

Name | Value
--- | ---
x | 1
y | 2

A table with one column:

| Only |
|------|
| row  |

Not a table (delimiter row column count differs):

| a | b |
|---|
| 1 | 2 |

## Task lists

- [ ] unchecked task
- [x] checked task
- [X] checked with capital X
- [ ] task with **formatting** and `code`
  - [ ] nested task
  - [x] nested done
1. [ ] ordered task
2. [x] ordered done

- [] not a task (no space)
- [ x] not a task

## Strikethrough

~~struck through~~ and ~single tilde~ and ~~**bold inside strike**~~.

Not struck: ~~ spaced ~~.

## Autolink literals

Visit www.example.com or https://example.com/path?q=1&r=2 or mail someone@example.com.

A trailing period is excluded: https://example.com/end.

## Footnotes

Here is a footnote reference[^1], another[^note], and a repeated one[^1].

[^1]: The first footnote.
[^note]: A footnote with several paragraphs.

    Indented second paragraph of the footnote, with `code`.

Reference to a missing footnote[^missing] stays literal.

## Alerts

> [!NOTE]
> Useful information that users should know.

> [!TIP]
> Helpful advice for doing things better.

> [!IMPORTANT]
> Key information users need to know.

> [!WARNING]
> Urgent info that needs immediate attention.

> [!CAUTION]
> Advises about risks or negative outcomes.

> [!note]
> Lowercase alert type.

> [!UNKNOWN]
> Not a known alert type; renders as a normal quote.

## Disallowed raw HTML (GFM tagfilter)

<title>should be escaped</title>
<textarea>should be escaped</textarea>
<style>body { color: red; }</style>
