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

    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }
}
