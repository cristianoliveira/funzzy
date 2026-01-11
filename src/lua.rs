use mlua::Lua;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[allow(dead_code)]
#[derive(Debug)]
pub struct LuaRuntime {
    lua: Lua,
    cache: HashMap<PathBuf, mlua::RegistryKey>,
}

#[allow(dead_code)]
impl LuaRuntime {
    pub fn new() -> mlua::Result<Self> {
        let lua = Lua::new();

        // Create sandboxed environment
        {
            let globals = lua.globals();

            // Remove dangerous modules/functions
            globals.set("io", mlua::Value::Nil)?;
            globals.set("os", mlua::Value::Nil)?;
            globals.set("package", mlua::Value::Nil)?;
            globals.set("debug", mlua::Value::Nil)?;

            // Remove dangerous functions from global namespace
            globals.set("load", mlua::Value::Nil)?;
            globals.set("loadfile", mlua::Value::Nil)?;
            globals.set("dofile", mlua::Value::Nil)?;
            globals.set("loadstring", mlua::Value::Nil)?;

            // Keep safe modules: string, table, math, coroutine (maybe)
            // These are generally safe
        }

        Ok(Self {
            lua,
            cache: HashMap::new(),
        })
    }

    pub fn evaluate_predicate(
        &mut self,
        script_path: &Path,
        event: &LuaEvent,
    ) -> mlua::Result<bool> {
        // Check cache first
        if let Some(key) = self.cache.get(script_path) {
            let func: mlua::Function = self.lua.registry_value(key)?;
            let result: mlua::Value = func.call(event.to_lua(&self.lua)?)?;
            return match result {
                mlua::Value::Boolean(b) => Ok(b),
                _ => Err(mlua::Error::RuntimeError(
                    "Predicate must return a boolean".to_string(),
                )),
            };
        }

        // Not in cache, load and compile
        let script = std::fs::read_to_string(script_path)
            .map_err(|e| mlua::Error::RuntimeError(format!("Failed to read Lua script: {}", e)))?;

        let func: mlua::Function = self.lua.load(&script).eval()?;

        // Store in registry and cache the key
        let func_clone = func.clone();
        let key = self.lua.create_registry_value(func_clone)?;
        self.cache.insert(script_path.to_path_buf(), key);

        let result: mlua::Value = func.call(event.to_lua(&self.lua)?)?;
        match result {
            mlua::Value::Boolean(b) => Ok(b),
            _ => Err(mlua::Error::RuntimeError(
                "Predicate must return a boolean".to_string(),
            )),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct LuaEvent {
    pub path: std::path::PathBuf,
    // Add more fields as needed: event type, metadata, etc.
}

#[allow(dead_code)]
impl LuaEvent {
    fn to_lua<'lua>(&self, lua: &'lua Lua) -> mlua::Result<mlua::Table<'lua>> {
        let table = lua.create_table()?;
        table.set("path", self.path.to_string_lossy().to_string())?;
        Ok(table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;

    #[test]
    fn test_sandbox_blocks_dangerous_functions() {
        let mut runtime = LuaRuntime::new().expect("Failed to create Lua runtime");

        // Test that dangerous functions are not accessible
        let test_script = r#"
            return function(event)
                -- Try to access dangerous functions
                if io ~= nil then
                    return false
                end
                if os ~= nil then
                    return false
                end
                if package ~= nil then
                    return false
                end
                if debug ~= nil then
                    return false
                end
                -- Should not be able to call load
                if load ~= nil then
                    return false
                end
                return true
            end
        "#;

        // Write to temp file
        let temp_dir = std::env::temp_dir();
        let script_path = temp_dir.join("test_sandbox.lua");
        let mut file = File::create(&script_path).expect("Failed to create temp file");
        file.write_all(test_script.as_bytes())
            .expect("Failed to write temp file");

        let event = LuaEvent {
            path: Path::new("/tmp/test.txt").to_path_buf(),
        };

        let result = runtime.evaluate_predicate(&script_path, &event);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);

        // Clean up
        std::fs::remove_file(&script_path).ok();
    }

    #[test]
    fn test_cache_works() {
        let mut runtime = LuaRuntime::new().expect("Failed to create Lua runtime");

        let test_script = r#"
            return function(event)
                return event.path:find("test") ~= nil
            end
        "#;

        let temp_dir = std::env::temp_dir();
        let script_path = temp_dir.join("test_cache.lua");
        let mut file = File::create(&script_path).expect("Failed to create temp file");
        file.write_all(test_script.as_bytes())
            .expect("Failed to write temp file");

        let event1 = LuaEvent {
            path: Path::new("/tmp/test1.txt").to_path_buf(),
        };
        let event2 = LuaEvent {
            path: Path::new("/tmp/other.txt").to_path_buf(),
        };

        // First evaluation should compile and cache
        let result1 = runtime.evaluate_predicate(&script_path, &event1);
        assert!(result1.is_ok());
        assert_eq!(result1.unwrap(), true);

        // Second evaluation should use cache
        let result2 = runtime.evaluate_predicate(&script_path, &event2);
        assert!(result2.is_ok());
        assert_eq!(result2.unwrap(), false);

        // Clean up
        std::fs::remove_file(&script_path).ok();
    }
}
