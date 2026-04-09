# unused_module_variable
## What it does
Checks for unused exported fields on static-table module variables.

## Why this is bad
Unused module fields can indicate dead API surface, stale state, or a misspelled export that consumers never read.

## Configuration
`ignore_fields` (default: `["__index"]`) - A list of additional static module field names that should be ignored. `__index` is always ignored by default. Entries may be written with or without a leading `.`, so `[".Attributes", ".Tag"]` and `["Attributes", "Tag"]` are equivalent.

## Example
```lua
local SomeModule = {}

SomeModule.UnusedVariable = 12

return SomeModule
```

## Remarks
If your modules intentionally expose write-only fields, you can ignore them in `selene.toml`.

```toml
[config.unused_module_variable]
ignore_fields = [".Attributes", ".Tag"]
```
