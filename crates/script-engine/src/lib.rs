//! JSON automation script parse / validate / helpers.

use riviu_core::AutomationScript;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScriptError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Validation(String),
}

pub fn parse_script(raw: &str) -> Result<AutomationScript, ScriptError> {
    let script: AutomationScript = serde_json::from_str(raw)?;
    validate(&script)?;
    Ok(script)
}

pub fn validate(script: &AutomationScript) -> Result<(), ScriptError> {
    if script.version != 1 {
        return Err(ScriptError::Validation(format!(
            "unsupported script version {}",
            script.version
        )));
    }
    if script.name.trim().is_empty() {
        return Err(ScriptError::Validation("script name required".into()));
    }
    if script.steps.is_empty() {
        return Err(ScriptError::Validation(
            "script needs at least one step".into(),
        ));
    }
    Ok(())
}

pub fn example_script_json() -> &'static str {
    r#"{
  "version": 1,
  "name": "Open app and capture",
  "steps": [
    { "action": "launchApp", "bundleId": "com.apple.Preferences" },
    { "action": "wait", "milliseconds": 2000 },
    { "action": "tap", "point": { "x": 195, "y": 400 } },
    { "action": "typeText", "value": "hello" },
    { "action": "screenshot", "name": "after-type" },
    { "action": "home" }
  ]
}"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_example() {
        let script = parse_script(example_script_json()).unwrap();
        assert_eq!(script.steps.len(), 6);
    }

    #[test]
    fn rejects_empty_steps() {
        let raw = r#"{"version":1,"name":"x","steps":[]}"#;
        assert!(parse_script(raw).is_err());
    }
}
