# Images

Local PNG (relative path):

![Sample image](assets/sample.png)

With a title:

![Sample with title](assets/sample.png "A 64×48 test image")

Relative with `./`:

![dot-slash](./assets/sample.png)

Inline among text: before ![inline](assets/sample.png) after.

Two in a row: ![one](assets/sample.png) ![two](assets/sample.png)

Reference style:

![ref image][sample]

[sample]: assets/sample.png

Image inside a link:

[![linked image](assets/sample.png)](https://example.com)

Image in a list:

- ![in list](assets/sample.png)
- text item

Image in a table:

| Image | Caption |
|-------|---------|
| ![cell](assets/sample.png) | in a cell |

Raw HTML image with size:

<img src="assets/sample.png" alt="html img" width="32">

Path with spaces (angle brackets):

![spaces](<assets/sample copy.png>)

Missing file (must show a placeholder, not crash):

![missing](assets/does-not-exist.png)

Not an image file:

![wrong type](../corpus/gfm.md)

Remote image (must not be fetched unless remote loading is enabled):

![remote](https://example.com/remote.png)

Empty alt and empty src:

![](assets/sample.png)

![empty src]()
