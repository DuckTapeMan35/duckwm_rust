use crate::proto::{self, value::Kind};

/// What a command returns.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Num(f64),
    Str(String),
    List(Vec<Value>),
    /// Fields in the order duckwm sent them.
    Map(Vec<(String, Value)>),
    /// Several return values.
    Multi(Vec<Value>),
}

impl From<proto::Value> for Value {
    fn from(v: proto::Value) -> Self {
        let list = |l: proto::ValueList| l.items.into_iter().map(Value::from).collect();
        match v.kind {
            None | Some(Kind::Nil(_)) => Value::Nil,
            Some(Kind::Boolean(b)) => Value::Bool(b),
            Some(Kind::Integer(i)) => Value::Int(i),
            Some(Kind::Number(n)) => Value::Num(n),
            Some(Kind::Str(s)) => Value::Str(s),
            Some(Kind::List(l)) => Value::List(list(l)),
            Some(Kind::Multi(l)) => Value::Multi(list(l)),
            Some(Kind::Map(m)) => Value::Map(
                m.fields
                    .into_iter()
                    .map(|f| (f.key, f.value.map(Value::from).unwrap_or(Value::Nil)))
                    .collect(),
            ),
        }
    }
}

impl From<Value> for proto::Value {
    fn from(v: Value) -> Self {
        let list = |items: Vec<Value>| proto::ValueList {
            items: items.into_iter().map(Into::into).collect(),
        };
        let kind = match v {
            Value::Nil => Kind::Nil(proto::Empty {}),
            Value::Bool(b) => Kind::Boolean(b),
            Value::Int(i) => Kind::Integer(i),
            Value::Num(n) => Kind::Number(n),
            Value::Str(s) => Kind::Str(s),
            Value::List(items) => Kind::List(list(items)),
            Value::Multi(items) => Kind::Multi(list(items)),
            Value::Map(fields) => Kind::Map(proto::ValueMap {
                fields: fields
                    .into_iter()
                    .map(|(key, v)| proto::ValueField {
                        key,
                        value: Some(v.into()),
                    })
                    .collect(),
            }),
        };
        proto::Value { kind: Some(kind) }
    }
}

macro_rules! from_int {
    ($($t:ty),*) => {$(
        impl From<$t> for Value {
            fn from(i: $t) -> Self {
                Value::Int(i as i64)
            }
        }
    )*};
}
from_int!(i32, i64, u32, u64, usize);

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Num(n)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(s)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.to_owned())
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(items: Vec<T>) -> Self {
        Value::List(items.into_iter().map(Into::into).collect())
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Nil, Into::into)
    }
}

impl Value {
    /// A map's field.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(items) | Value::Multi(items) => Some(items),
            _ => None,
        }
    }

    /// A list of ids, e.g. an arranger's window order kept in its state.
    pub fn as_ids(&self) -> Option<Vec<u32>> {
        self.as_list()?
            .iter()
            .map(|v| v.as_int().map(|i| i as u32))
            .collect()
    }

    pub fn as_num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_protobuf() {
        let v = Value::Map(vec![
            (
                "tree".into(),
                Value::List(vec![1u32.into(), "h".into(), Value::Nil]),
            ),
            ("h".into(), true.into()),
            ("ratio".into(), 0.5.into()),
        ]);
        assert_eq!(Value::from(proto::Value::from(v.clone())), v);
    }

    #[test]
    fn ids() {
        assert_eq!(Value::from(vec![3u32, 1, 2]).as_ids(), Some(vec![3, 1, 2]));
        assert_eq!(Value::from("x").as_ids(), None);
    }
}
