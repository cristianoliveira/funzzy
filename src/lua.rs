use mlua::Lua;
use std::path::Path;

pub struct LuaRuntime {
    lua: Lua,
}

impl LuaRuntime {
    pub fn new() -> mlua::Result<Self> {
        let lua = Lua::new();
        Ok(Self { lua })
    }

    pub fn evaluate_predicate(&self, script_path: &Path, event: &LuaEvent) -> mlua::Result<bool> {
        let script = std::fs::read_to_string(script_path)
            .map_err(|e| mlua::Error::RuntimeError(format!("Failed to read Lua script: {}", e)))?;

        let func: mlua::Function = self.lua.load(&script).eval()?;
        let result: mlua::Value = func.call(event.to_lua(&self.lua)?)?;
        match result {
            mlua::Value::Boolean(b) => Ok(b),
            _ => Err(mlua::Error::RuntimeError(
                "Predicate must return a boolean".to_string(),
            )),
        }
    }
}

pub struct LuaEvent {
    pub path: std::path::PathBuf,
    // Add more fields as needed: event type, metadata, etc.
}

impl LuaEvent {
    fn to_lua<'lua>(&self, lua: &'lua Lua) -> mlua::Result<mlua::Table<'lua>> {
        let table = lua.create_table()?;
        table.set("path", self.path.to_string_lossy().to_string())?;
        Ok(table)
    }
}
