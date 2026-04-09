local SomeModule = {}

SomeModule.UnusedVariable = 12
SomeModule.__index = SomeModule
SomeModule.Attributes = {}
SomeModule.Tag = "example"

return SomeModule
