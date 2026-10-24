use crate::proto;
use crate::{Cmd, Value};

/// Commands to run in duckwm's transaction; for an arranger, its layout and
/// next state; for a factory, the arranger it made.
#[derive(Debug, Default, Clone)]
pub struct Reply {
    pub commands: Vec<Cmd>,
    pub layout: Option<proto::LayoutSpec>,
    /// None leaves the state as it is; Some(Value::Nil) clears it.
    pub state: Option<Value>,
    pub made: Option<u64>,
}

impl Reply {
    /// Nothing to add: the transaction just goes on.
    pub fn none() -> Self {
        Self::default()
    }

    /// Commands to run as part of the transaction that called us.
    pub fn commands(commands: impl IntoIterator<Item = Cmd>) -> Self {
        Self {
            commands: commands.into_iter().collect(),
            ..Self::default()
        }
    }

    pub(crate) fn into_return(self) -> proto::Return {
        proto::Return {
            commands: self.commands.into_iter().map(Into::into).collect(),
            err: None,
            callback: self.made,
            layout: self.layout,
            state: self.state.map(Into::into),
        }
    }
}

impl From<()> for Reply {
    fn from(_: ()) -> Self {
        Self::none()
    }
}

impl From<Cmd> for Reply {
    fn from(c: Cmd) -> Self {
        Self::commands([c])
    }
}

impl From<Vec<Cmd>> for Reply {
    fn from(c: Vec<Cmd>) -> Self {
        Self::commands(c)
    }
}
