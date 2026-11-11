//! Query-string encoding shared by [`Transport`](super::transport::Transport),
//! the [pager](crate::clients::pagination::Pager) and the Python bindings.
//!
//! Query parameters are a JSON object. Each member becomes zero or more
//! `key=value` pairs:
//!
//! | Value | Pairs sent |
//! | --- | --- |
//! | string | one pair with the string, also an empty one (`key=`) |
//! | number | one pair with its JSON text (`10`, `-1.5`) |
//! | boolean | one pair, `true` or `false` |
//! | `null` | none: the parameter is left out |
//! | list of strings, numbers and booleans | one pair per item, in list order (`tag_key=a&tag_key=b`); none for an empty list |
//!
//! Other values are an error, and no request is sent: objects, nested lists
//! and `null` inside a list. TES matches `tag_key` and `tag_value` by
//! position, so leaving out a `null` item would pair the wrong keys and values.
//!
//! Pairs follow the order of the object's keys (sorted, as `serde_json` keeps
//! them). Keys and values are encoded as
//! `application/x-www-form-urlencoded`: a space becomes `+`, and every byte
//! except ASCII letters, digits and `*-._` is percent-encoded as UTF-8.
use serde_json::Value;
use std::{error::Error, fmt};

/// Query parameters that the shared policy can't encode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryError {
    /// The parameters are not a JSON object.
    NotAnObject,
    /// A parameter is an object or a list with an object, list or `null`.
    UnsupportedValue {
        /// The name of the parameter.
        key: String,
    },
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnObject => write!(f, "query parameters must be a JSON object"),
            Self::UnsupportedValue { key } => write!(
                f,
                "query parameter '{key}' must be a string, number, boolean, null \
                 or a list of strings, numbers and booleans"
            ),
        }
    }
}

impl Error for QueryError {}

impl From<QueryError> for std::io::Error {
    fn from(error: QueryError) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, error)
    }
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

/// Converts query parameters to `(key, value)` pairs, following the
/// [module's policy](self).
///
/// # Errors
///
/// [`QueryError`] if `params` isn't an object or holds an unsupported value.
pub fn query_pairs(params: &Value) -> Result<Vec<(String, String)>, QueryError> {
    let object = params.as_object().ok_or(QueryError::NotAnObject)?;
    let mut pairs = Vec::new();
    for (key, value) in object {
        let items = match value {
            Value::Null => continue,
            Value::Array(items) => items.as_slice(),
            value => std::slice::from_ref(value),
        };
        for item in items {
            let text =
                scalar(item).ok_or_else(|| QueryError::UnsupportedValue { key: key.clone() })?;
            pairs.push((key.clone(), text));
        }
    }
    Ok(pairs)
}

/// Encodes query parameters as a query string without the leading `?`,
/// following the [module's policy](self). Parameters without pairs give an
/// empty string.
///
/// # Errors
///
/// [`QueryError`] if `params` isn't an object or holds an unsupported value.
pub fn encode_query(params: &Value) -> Result<String, QueryError> {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in query_pairs(params)? {
        query.append_pair(&key, &value);
    }
    Ok(query.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn encodes_the_documented_cases() {
        let params = json!({
            "a b": "c",
            "e": [],
            "f": -1.5,
            "n": null,
            "q": "x&y=z/é+%?#*-._~",
            "s": "",
            "t": [true, 1, "x"],
        });
        assert_eq!(
            encode_query(&params).unwrap(),
            "a+b=c&f=-1.5&q=x%26y%3Dz%2F%C3%A9%2B%25%3F%23*-._%7E&s=&t=true&t=1&t=x"
        );
        assert_eq!(encode_query(&json!({"n": null})).unwrap(), "");
    }

    #[test]
    fn rejects_unsupported_values() {
        for params in [
            json!({"k": {"a": 1}}),
            json!({"k": [["a"]]}),
            json!({"k": [{"a": 1}]}),
            json!({"k": ["a", null]}),
        ] {
            assert_eq!(
                query_pairs(&params),
                Err(QueryError::UnsupportedValue { key: "k".into() }),
                "{params}"
            );
        }
        for params in [json!(null), json!([]), json!("a=b")] {
            assert_eq!(query_pairs(&params), Err(QueryError::NotAnObject));
        }
    }

    #[test]
    fn converts_to_an_invalid_input_io_error() {
        let error = std::io::Error::from(QueryError::NotAnObject);
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "query parameters must be a JSON object");
    }
}
