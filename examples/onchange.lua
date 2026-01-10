-- Simple Lua predicate that returns true if path contains "trigger"
return function(event)
    return event.path:find("trigger") ~= nil
end