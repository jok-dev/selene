# recursive_require
## What it does
Checks for `require` calls that eventually require the current module again.

## Why this is bad
Recursive module dependencies are fragile and can lead to partially initialized modules being observed at runtime.

## Configuration
This lint has no extra configuration.

## Example
```lua
local sibling = require(script.Parent.Sibling)
```

If `Sibling` directly or indirectly requires the current module, selene will emit a warning on that `require`.
