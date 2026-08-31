use std::fmt;

/// A UI action failure classified by whether a public effect may already have escaped.
#[derive(Debug)]
pub enum ActionFailure {
    BeforeEffect(anyhow::Error),
    AfterEffect(anyhow::Error),
}

impl ActionFailure {
    pub fn before(error: impl Into<anyhow::Error>) -> Self {
        Self::BeforeEffect(error.into())
    }

    pub fn after(error: impl Into<anyhow::Error>) -> Self {
        Self::AfterEffect(error.into())
    }

    pub fn effect_may_have_gone_out(&self) -> bool {
        matches!(self, Self::AfterEffect(_))
    }
}

impl fmt::Display for ActionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BeforeEffect(error) | Self::AfterEffect(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ActionFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BeforeEffect(error) | Self::AfterEffect(error) => Some(error.as_ref()),
        }
    }
}
