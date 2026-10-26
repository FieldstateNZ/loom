//! Explicit host-side validation for the two source-reviewed memory tools.
use crate::Error;
use serde_json::Value;

pub(super) fn validate(name: &str, input: &Value) -> Result<(), Error> {
    let values = input.as_object().ok_or(Error::Tool)?;
    let keys: &[&str] = match name {
        "memory_recall" => &["query", "type", "tags", "limit", "includeShared"],
        "memory_search" => &[
            "type", "tags", "pinned", "vault", "search", "limit", "page", "scope", "sort",
        ],
        _ => return Err(Error::Tool),
    };
    if values.keys().any(|k| !keys.contains(&k.as_str())) {
        return Err(Error::Tool);
    }
    let text = |key: &str, max: usize| {
        values.get(key).is_none_or(|v| {
            v.as_str()
                .is_some_and(|s| !s.trim().is_empty() && s.len() <= max)
        })
    };
    let choice = |key: &str, allowed: &[&str]| {
        values
            .get(key)
            .is_none_or(|v| v.as_str().is_some_and(|s| allowed.contains(&s)))
    };
    if !choice(
        "type",
        &["note", "fact", "decision", "preference", "insight"],
    ) || !choice("scope", &["own", "shared", "all"])
        || !choice(
            "sort",
            &[
                "newest",
                "oldest",
                "recently_recalled",
                "most_recalled",
                "confidence_desc",
            ],
        )
        || ["pinned", "vault", "includeShared"]
            .iter()
            .any(|k| values.get(*k).is_some_and(|v| !v.is_boolean()))
        || values.get("limit").is_some_and(|v| {
            v.as_u64()
                .is_none_or(|n| n == 0 || n > if name == "memory_recall" { 20 } else { 50 })
        })
        || values
            .get("page")
            .is_some_and(|v| v.as_u64().is_none_or(|n| n == 0 || n > 100))
        || !text("search", 2000)
    {
        return Err(Error::Tool);
    }
    if name == "memory_recall" {
        if !values.contains_key("query")
            || !text("query", 2000)
            || values.get("tags").is_some_and(|v| {
                v.as_array().is_none_or(|tags| {
                    tags.len() > 20
                        || tags
                            .iter()
                            .any(|t| t.as_str().is_none_or(|s| s.len() > 100))
                })
            })
        {
            return Err(Error::Tool);
        }
    } else if !text("tags", 2000) {
        return Err(Error::Tool);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn reviewed_arguments_and_bounds() {
        for (name, args) in [
            (
                "memory_recall",
                json!({"query":"What tea do I prefer?","limit":20,"tags":["preference"]}),
            ),
            (
                "memory_search",
                json!({"search":"tea","page":1,"scope":"own","limit":50}),
            ),
        ] {
            assert!(validate(name, &args).is_ok());
        }
        for (name, args) in [
            ("capture_item", json!({"title":"No writes"})),
            ("memory_recall", json!({})),
            ("memory_search", json!({"workspaceId":"other"})),
            ("memory_search", json!({"limit":-1})),
            ("memory_search", json!({"limit":1.5})),
            ("memory_recall", json!({"query":"x","limit":21})),
            ("memory_recall", json!({"query":"x","tags":"wrong"})),
            ("memory_search", json!({"tags":["wrong"]})),
            ("memory_search", json!({"pinned":"true"})),
            ("memory_search", json!({"page":0})),
            ("memory_search", json!({"sort":"arbitrary"})),
        ] {
            assert!(validate(name, &args).is_err(), "{name}: {args}");
        }
    }
}
