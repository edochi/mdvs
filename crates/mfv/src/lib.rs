use gray_matter::Matter;
use gray_matter::engine::YAML;
use serde_json::Value;

/// Extract frontmatter and body from markdown content.
/// Detects TOML (`+++`) vs YAML (`---`) delimiters.
pub fn extract_frontmatter(content: &str) -> (Option<Value>, String) {
    let trimmed = content.trim_start();

    if trimmed.starts_with("+++") {
        extract_toml_frontmatter(content)
    } else {
        extract_yaml_frontmatter(content)
    }
}

fn extract_toml_frontmatter(content: &str) -> (Option<Value>, String) {
    let trimmed = content.trim_start();
    let after_open = &trimmed[3..];
    let Some(close_pos) = after_open.find("+++") else {
        return (None, content.to_string());
    };

    let toml_str = &after_open[..close_pos];
    let body = after_open[close_pos + 3..]
        .trim_start_matches('\n')
        .to_string();

    match toml_str.parse::<toml::Value>() {
        Ok(toml_val) => {
            let json = toml_to_json(&toml_val);
            (Some(json), body)
        }
        Err(_) => (None, content.to_string()),
    }
}

fn toml_to_json(val: &toml::Value) -> Value {
    match val {
        toml::Value::String(s) => Value::String(s.clone()),
        toml::Value::Integer(i) => Value::Number((*i).into()),
        toml::Value::Float(f) => {
            serde_json::Number::from_f64(*f).map_or(Value::Null, Value::Number)
        }
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Datetime(dt) => Value::String(dt.to_string()),
        toml::Value::Array(arr) => Value::Array(arr.iter().map(toml_to_json).collect()),
        toml::Value::Table(map) => {
            let obj: serde_json::Map<String, Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), toml_to_json(v)))
                .collect();
            Value::Object(obj)
        }
    }
}

fn extract_yaml_frontmatter(content: &str) -> (Option<Value>, String) {
    let matter = Matter::<YAML>::new();
    let parsed = matter.parse(content);

    let data = parsed.data.and_then(|d| {
        let yaml: serde_yaml::Value = d.deserialize().ok()?;
        let json = yaml_to_json(&yaml);
        if json.is_object() { Some(json) } else { None }
    });

    (data, parsed.content)
}

fn yaml_to_json(val: &serde_yaml::Value) -> Value {
    match val {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(b) => Value::Bool(*b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Number(i.into())
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)
            } else {
                Value::Null
            }
        }
        serde_yaml::Value::String(s) => Value::String(s.clone()),
        serde_yaml::Value::Sequence(seq) => Value::Array(seq.iter().map(yaml_to_json).collect()),
        serde_yaml::Value::Mapping(map) => {
            let obj: serde_json::Map<String, Value> = map
                .iter()
                .map(|(k, v)| {
                    let key = match k {
                        serde_yaml::Value::String(s) => s.clone(),
                        other => format!("{other:?}"),
                    };
                    (key, yaml_to_json(v))
                })
                .collect();
            Value::Object(obj)
        }
        serde_yaml::Value::Tagged(tagged) => yaml_to_json(&tagged.value),
    }
}
