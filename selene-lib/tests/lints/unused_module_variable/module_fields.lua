local SomeModule = {}

SomeModule.UnusedVariable = 12
SomeModule.__index = SomeModule
SomeModule.UsedFieldInModule = 34
SomeModule.UsedFieldInOtherModule = 56

local _ = SomeModule.UsedFieldInModule

return SomeModule
