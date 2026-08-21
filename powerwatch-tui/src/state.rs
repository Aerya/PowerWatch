#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunState {
    #[default]
    Running,
    Paused,
}

impl RunState {
    pub fn toggle(self) -> Self {
        match self {
            RunState::Running => RunState::Paused,
            RunState::Paused => RunState::Running,
        }
    }

    pub fn is_paused(self) -> bool {
        self == RunState::Paused
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoggingState {
    #[default]
    Idle,
    Logging,
}

impl LoggingState {
    pub fn toggle(self) -> Self {
        match self {
            LoggingState::Idle => LoggingState::Logging,
            LoggingState::Logging => LoggingState::Idle,
        }
    }

    pub fn is_logging(self) -> bool {
        self == LoggingState::Logging
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SuggestionsState {
    #[default]
    Closed,
    Open,
    Confirming,
}

impl SuggestionsState {
    pub fn is_open(self) -> bool {
        matches!(self, SuggestionsState::Open | SuggestionsState::Confirming)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_running_by_default() {
        assert_eq!(RunState::default(), RunState::Running);
        assert!(!RunState::default().is_paused());
    }

    #[test]
    fn toggling_running_gives_paused() {
        assert_eq!(RunState::Running.toggle(), RunState::Paused);
    }

    #[test]
    fn toggling_paused_gives_running() {
        assert_eq!(RunState::Paused.toggle(), RunState::Running);
    }

    #[test]
    fn is_paused_reflects_the_current_state() {
        assert!(RunState::Paused.is_paused());
        assert!(!RunState::Running.is_paused());
    }

    #[test]
    fn logging_starts_idle_by_default() {
        assert_eq!(LoggingState::default(), LoggingState::Idle);
        assert!(!LoggingState::default().is_logging());
    }

    #[test]
    fn toggling_idle_gives_logging() {
        assert_eq!(LoggingState::Idle.toggle(), LoggingState::Logging);
    }

    #[test]
    fn toggling_logging_gives_idle() {
        assert_eq!(LoggingState::Logging.toggle(), LoggingState::Idle);
    }

    #[test]
    fn suggestions_state_starts_closed() {
        assert_eq!(SuggestionsState::default(), SuggestionsState::Closed);
        assert!(!SuggestionsState::default().is_open());
    }

    #[test]
    fn is_open_returns_true_for_open_and_confirming() {
        assert!(SuggestionsState::Open.is_open());
        assert!(SuggestionsState::Confirming.is_open());
        assert!(!SuggestionsState::Closed.is_open());
    }
}
