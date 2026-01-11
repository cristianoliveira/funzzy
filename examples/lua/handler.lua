-- Simple Lua predicate that returns true if path contains "trigger"
return function(event)
    print("Lua predicate called with path: " .. event.path)
    local pos = event.path:find("trigger")
    print("Position: " .. tostring(pos))
    local result = pos ~= nil
    print("Result: " .. tostring(result))
    return result
end