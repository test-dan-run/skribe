//! The lifecycle states as the frontend sees them. They mirror the domain's enums one for
//! one; the exhaustive conversions below stop compiling if either side gains a state.

use serde::Serialize;

macro_rules! mirror_state {
    ($(#[$meta:meta])* $name:ident { $($variant:ident),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
        #[cfg_attr(test, derive(ts_rs::TS))]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }

        impl From<skribe_domain::$name> for $name {
            fn from(state: skribe_domain::$name) -> Self {
                match state {
                    $(skribe_domain::$name::$variant => Self::$variant),+
                }
            }
        }
    };
}

mirror_state!(
    /// Where a meeting is between preparation and a finalised recording.
    RecordingState {
        Setup,
        Ready,
        Recording,
        Finalising,
        Recorded,
        Interrupted,
        Failed,
    }
);

mirror_state!(
    /// Where a meeting's local processing is.
    ProcessingState {
        None,
        WaitingForModels,
        Queued,
        Running,
        Review,
        Cancelled,
        Failed,
        Interrupted,
    }
);

mirror_state!(
    /// Where a meeting's minutes are.
    MinutesState {
        None,
        Generating,
        Current,
        Stale,
        CandidateAvailable,
        Cancelled,
        Failed,
    }
);

mirror_state!(
    /// Which of a recording's two original tracks a track is.
    TrackSource {
        Microphone,
        System,
    }
);

mirror_state!(
    /// Whether a track's file has been completed.
    TrackFinalisation {
        Pending,
        Finalised,
        Recovered,
        Failed,
    }
);

mirror_state!(
    /// How far a track can be relied on.
    TrackHealth {
        Healthy,
        Degraded,
        Failed,
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    fn wire<T: Serialize>(value: T) -> String {
        serde_json::to_value(value)
            .expect("serialise")
            .as_str()
            .expect("a string")
            .to_owned()
    }

    #[test]
    fn states_cross_the_command_boundary_under_their_stored_names() {
        for state in skribe_domain::RecordingState::ALL {
            assert_eq!(wire(RecordingState::from(*state)), state.as_str());
        }
        for state in skribe_domain::ProcessingState::ALL {
            assert_eq!(wire(ProcessingState::from(*state)), state.as_str());
        }
        for state in skribe_domain::MinutesState::ALL {
            assert_eq!(wire(MinutesState::from(*state)), state.as_str());
        }
        for state in skribe_domain::TrackSource::ALL {
            assert_eq!(wire(TrackSource::from(*state)), state.as_str());
        }
        for state in skribe_domain::TrackFinalisation::ALL {
            assert_eq!(wire(TrackFinalisation::from(*state)), state.as_str());
        }
        for state in skribe_domain::TrackHealth::ALL {
            assert_eq!(wire(TrackHealth::from(*state)), state.as_str());
        }
    }
}
