//! Pure configuration for product-owned Flow extensions.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransformConfig {
    pub name: String,
    pub source: String,
    pub operation: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub pattern: Option<String>,
}
impl TransformConfig {
    pub fn validate(&self) -> Result<(), String> {
        super::validate_flow_variable_name(&self.name).map_err(str::to_owned)?;
        super::validate_flow_variable_name(&self.source).map_err(str::to_owned)?;
        if ![
            "trim",
            "lowercase",
            "uppercase",
            "splitLines",
            "firstLine",
            "jsonGet",
            "replace",
            "regexExtract",
            "joinLines",
        ]
        .contains(&self.operation.as_str())
        {
            return Err("unknown transformation".into());
        }
        if [&self.path, &self.value, &self.pattern]
            .iter()
            .any(|v| v.as_ref().is_some_and(|s| s.len() > 4096))
        {
            return Err("transform parameter too large".into());
        }
        if self.operation == "jsonGet"
            && self
                .path
                .as_deref()
                .is_none_or(|p| !p.is_empty() && !p.starts_with('/'))
        {
            return Err("JSON path must be an RFC6901 pointer".into());
        }
        if self.operation == "regexExtract" {
            regex::RegexBuilder::new(self.pattern.as_deref().ok_or("regex pattern required")?)
                .size_limit(256 * 1024)
                .build()
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn execute(&self, source: &str) -> Result<String, String> {
        self.validate()?;
        let value = match self.operation.as_str() {
            "trim" => source.trim().to_owned(),
            "lowercase" => source.to_lowercase(),
            "uppercase" => source.to_uppercase(),
            "splitLines" => serde_json::to_string(&source.lines().collect::<Vec<_>>())
                .map_err(|e| e.to_string())?,
            "firstLine" => source.split('\n').next().unwrap_or("").trim().to_owned(),
            "joinLines" => source
                .split('\n')
                .map(str::trim)
                .collect::<Vec<_>>()
                .join(self.value.as_deref().unwrap_or(",")),
            "replace" => source.replace(
                self.pattern.as_deref().ok_or("replace pattern required")?,
                self.value.as_deref().unwrap_or(""),
            ),
            "regexExtract" => {
                let pattern =
                    regex::RegexBuilder::new(self.pattern.as_deref().ok_or("pattern required")?)
                        .size_limit(256 * 1024)
                        .build()
                        .map_err(|e| e.to_string())?;
                pattern
                    .captures(source)
                    .and_then(|c| c.get(1).or_else(|| c.get(0)))
                    .map(|m| m.as_str().to_owned())
                    .unwrap_or_default()
            }
            "jsonGet" => {
                let parsed: serde_json::Value =
                    serde_json::from_str(source).map_err(|e| e.to_string())?;
                let result = parsed
                    .pointer(self.path.as_deref().unwrap_or(""))
                    .ok_or("JSON pointer did not match")?;
                result
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| result.to_string())
            }
            _ => return Err("unknown transform".into()),
        };
        if value.chars().count() > 4096 {
            return Err("transform output too large".into());
        }
        Ok(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrReadConfig {
    pub name: String,
    #[serde(default)]
    pub region: Option<super::VisionRegion>,
    #[serde(default = "default_languages")]
    pub languages: Vec<String>,
    #[serde(default = "default_confidence")]
    pub min_confidence: f64,
}
fn default_languages() -> Vec<String> {
    vec!["vi".into(), "en".into()]
}
fn default_confidence() -> f64 {
    0.5
}
impl OcrReadConfig {
    pub fn validate(&self) -> Result<(), String> {
        super::validate_flow_variable_name(&self.name).map_err(str::to_owned)?;
        if !self.min_confidence.is_finite() || !(0.0..=1.0).contains(&self.min_confidence) {
            return Err("OCR confidence must be 0..1".into());
        }
        if self.languages.is_empty()
            || self.languages.len() > 2
            || self.languages.iter().any(|l| l != "en" && l != "vi")
        {
            return Err("OCR languages must contain en and/or vi".into());
        }
        if let Some(region) = &self.region {
            super::validate_vision_region(region)?;
        }
        Ok(())
    }
}

pub fn output_variable(config: &super::CompiledActionConfig) -> Option<&str> {
    use super::CompiledActionConfig as C;
    match config {
        C::SetVariable { name, .. }
        | C::ReadText { name, .. }
        | C::CopyVariable { name, .. }
        | C::OcrReadText { name, .. } => Some(name),
        C::FileRead(c) => Some(&c.name),
        C::Transform(c) => Some(&c.name),
        C::FileWrite(c) => Some(&c.name),
        C::HttpRequest(c) => Some(&c.name),
        C::SheetRead(c) => Some(&c.name),
        C::SheetWrite(c) => Some(&c.name),
        _ => None,
    }
}

/// Literal text substitution only; variable values are never evaluated as expressions.
pub fn resolve_text(
    template: &str,
    values: &std::collections::BTreeMap<String, String>,
) -> Result<String, String> {
    let mut rest = template;
    let mut out = String::new();
    while let Some((prefix, suffix)) = rest.split_once("${") {
        out.push_str(prefix);
        let (name, after) = suffix
            .split_once('}')
            .ok_or("unterminated variable reference")?;
        super::validate_flow_variable_name(name).map_err(str::to_owned)?;
        out.push_str(
            values
                .get(name)
                .ok_or_else(|| format!("variable {name} has no completed writer"))?,
        );
        if out.chars().count() > 4096 {
            return Err("resolved value exceeds 4096 characters".into());
        }
        rest = after;
    }
    out.push_str(rest);
    if out.chars().count() > 4096 {
        return Err("resolved value exceeds 4096 characters".into());
    }
    Ok(out)
}

pub fn referenced_variables(template: &str) -> Result<Vec<String>, String> {
    let mut rest = template;
    let mut names = Vec::new();
    while let Some((_, suffix)) = rest.split_once("${") {
        let (name, after) = suffix
            .split_once('}')
            .ok_or("unterminated variable reference")?;
        super::validate_flow_variable_name(name).map_err(str::to_owned)?;
        names.push(name.into());
        rest = after;
    }
    Ok(names)
}
