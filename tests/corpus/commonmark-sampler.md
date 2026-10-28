# CommonMark sampler

Every block and inline construct from the CommonMark spec, at least once.

## Headings

# ATX heading 1
## ATX heading 2
### ATX heading 3
#### ATX heading 4
##### ATX heading 5
###### ATX heading 6

### Closed ATX heading ###

Setext heading 1
================

Setext heading 2
----------------

#hashtag is not a heading (no space)

\# escaped hash is not a heading

## Paragraphs and line breaks

A paragraph spans
several source lines
and is joined with soft breaks.

Hard break with two trailing spaces  
next line.

Hard break with a backslash\
next line.

## Thematic breaks

***
---
___
 * * *

## Block quotes

> A block quote.
> With two lines.
>
> > A nested quote.
>
> - A list inside a quote
> - Second item

> Lazy continuation
of a block quote.

## Lists

- Bullet with dash
- Second item
  - Nested item
    - Deeper item
- Third item

* Star bullet
+ Plus bullet (starts a new list)

1. Ordered
2. Second
3. Third

7. Starts at seven
8. Eight

1) Paren delimiter
2) Second

- Loose list item one

- Loose list item two

  With a second paragraph.

- Item with code:

      indented code inside a list item

## Code

Inline `code`, ``code with ` backtick``, and `` `leading` ``.

    indented code block
    line two

```
fenced, no info string
```

```python
def hello():
    return "world"
```

~~~
tilde fence
~~~

## HTML blocks

<div class="note">
<p>Raw HTML block.</p>
</div>

<!-- an HTML comment -->

Inline <span>HTML span</span> and <kbd>Ctrl</kbd>+<kbd>S</kbd>.

## Link reference definitions

[ref]: https://example.com/ref "Reference title"
[Other Ref]: <https://example.com/with spaces> 'single quoted'

## Emphasis

*em*, _em_, **strong**, __strong__, ***strong em***, **_mixed_**, *nested **strong** inside*.

intra*word*emphasis, but snake_case_words stay plain.

## Links

[inline link](https://example.com), [with title](https://example.com "Title"),
[reference link][ref], [collapsed][], [shortcut], [case-insensitive][other ref].

[collapsed]: https://example.com/collapsed
[shortcut]: https://example.com/shortcut

Autolinks: <https://example.com/auto> and <mailto:someone@example.com> and <someone@example.com>.

Relative link: [another file](./gfm.md) and [anchor](#emphasis).

## Images

![alt text](assets/sample.png "Sample image")

![reference image][img]

[img]: assets/sample.png

[![image inside a link](assets/sample.png)](https://example.com)

## Escapes and entities

\*not emphasis\*, \`not code\`, \[not a link\], \\ backslash.

Entities: &copy; &amp; &lt; &gt; &quot; &#35; &#x1F600; &nbsp;end.

## Edge cases

Trailing spaces in a final line are not a hard break.  
