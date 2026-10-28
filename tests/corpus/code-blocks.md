# Code block edge cases

Cases from docs/code-blocks.md §5. This file contains real TAB characters; keep them.

## Tab-indented content with an info string and attributes

```rust title="main.rs"
fn main() {
	println!("tab-indented");
		// two tabs
}
```

## Tilde fence containing a backtick fence

~~~~
```
nested backtick fence is content
```
~~~~

## Longer backtick fence containing a shorter one

`````markdown
````
```
three levels
```
````
`````

## Indented fence (indent stripped from content)

  ```
  indented fence: two spaces stripped
    four spaces become two
  ```

## Fence inside a list item

- list item

  ```
  fenced code inside a list
  ```

- second item with a tab-indented line inside its fence

  ```sh
  echo "start"
	echo "tab in list fence"
  ```

1. ordered item
   ```json
   {"key": [1, 2, 3]}
   ```

## Fence inside a block quote

> ```python
> print("quoted code")
> ```

## Two adjacent blocks with no blank line between

```js
const a = 1;
```
```ts
const b: number = 2;
```

## Closing fence longer than the opening

```
closed by a longer fence
`````

## Info string variants

```  c++  
int main() { return 0; }
```

```{.haskell .numberLines}
main = putStrLn "pandoc attrs"
```

~~~ backticks`in`tilde-info
tilde info strings may contain backticks
~~~

## Not a fence: backticks in the info string

``` not`a`fence
this is a paragraph, not code
```

## Empty code block

```
```

## Blank lines and trailing whitespace inside code

```text
line one


line four (two blank lines above)   
	
```

## Indented code block with tabs

	tab-indented code block
		second level

## Unclosed fence at the end

```unclosed
runs to end of document
no closing fence follows
