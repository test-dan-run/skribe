//! The three lifecycle state machines of a meeting. They are stored and changed separately;
//! the single flow in AGENTS.md is a view derived from the three.

/// Defines a state enum with one stable text form per state, used in the database and across
/// the command boundary.
macro_rules! define_state {
    ($(#[$meta:meta])* $name:ident {
        $($(#[$variant_meta:meta])* $variant:ident => $text:literal),+ $(,)?
    }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$variant_meta])* $variant),+
        }

        impl $name {
            pub const ALL: &[Self] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }

            pub fn parse(text: &str) -> Option<Self> {
                match text {
                    $($text => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

define_state!(
    /// Where a meeting is between preparation and a finalised recording.
    RecordingState {
        Setup => "setup",
        Ready => "ready",
        Recording => "recording",
        Finalising => "finalising",
        Recorded => "recorded",
        Interrupted => "interrupted",
        Failed => "failed",
    }
);

impl RecordingState {
    /// Whether the state machine has an edge from `self` to `next`. There is no pause, so no
    /// edge leaves and re-enters `Recording`.
    pub fn can_become(self, next: Self) -> bool {
        use RecordingState::{Failed, Finalising, Interrupted, Ready, Recorded, Recording, Setup};
        matches!(
            (self, next),
            // Preflight passed and the tracks are registered.
            (Setup, Ready)
                // Capture could not start; nothing was recorded.
                | (Ready, Setup)
                | (Ready, Recording)
                | (Recording, Finalising)
                | (Finalising, Recorded | Failed)
                // Found at startup with no live session and audio on disk.
                | (Recording | Finalising, Interrupted)
                // Recovery rebuilt the tracks from their journals, or could not.
                | (Interrupted, Recorded | Failed)
        )
    }

    /// The agenda can change only before recording starts.
    pub fn allows_preparation(self) -> bool {
        self == Self::Setup
    }

    /// A capture session owns the meeting: it is starting, recording or finalising.
    pub fn is_live(self) -> bool {
        matches!(self, Self::Ready | Self::Recording | Self::Finalising)
    }

    /// Where a recording ends up once every track has been finalised or recovered, or has
    /// failed to be. It is `Recorded` only when each track left a valid file, which includes
    /// the valid partial file of a track that stopped early. Otherwise it is `Failed`; the
    /// tracks that are valid, the notes and the events are kept either way.
    pub fn after_finalisation(tracks: &[TrackFinalisation]) -> Self {
        let all_valid = !tracks.is_empty() && tracks.iter().all(|track| track.has_valid_file());
        if all_valid {
            Self::Recorded
        } else {
            Self::Failed
        }
    }
}

define_state!(
    /// Which of a recording's two original tracks a track is.
    TrackSource {
        Microphone => "microphone",
        System => "system",
    }
);

define_state!(
    /// Whether a track's file has been completed.
    TrackFinalisation {
        /// Capture has not finished, or was interrupted and has not been recovered.
        Pending => "pending",
        /// The track was stopped and finalised in the ordinary way.
        Finalised => "finalised",
        /// The recording was interrupted and the track was rebuilt from its journal.
        Recovered => "recovered",
        /// There is no valid file for this track.
        Failed => "failed",
    }
);

impl TrackFinalisation {
    pub fn has_valid_file(self) -> bool {
        matches!(self, Self::Finalised | Self::Recovered)
    }
}

define_state!(
    /// How far a track can be relied on.
    TrackHealth {
        /// Captured from start to stop with nothing lost.
        Healthy => "healthy",
        /// A valid track that is known to be missing some audio.
        Degraded => "degraded",
        /// The track stopped before the recording did, or has no valid file.
        Failed => "failed",
    }
);

define_state!(
    /// Where a meeting's local processing is. Transitions arrive with the pipeline.
    ProcessingState {
        None => "none",
        WaitingForModels => "waiting_for_models",
        Queued => "queued",
        Running => "running",
        Review => "review",
        Cancelled => "cancelled",
        Failed => "failed",
        Interrupted => "interrupted",
    }
);

define_state!(
    /// Where a meeting's minutes are. Transitions arrive with minutes generation.
    MinutesState {
        None => "none",
        Generating => "generating",
        Current => "current",
        Stale => "stale",
        CandidateAvailable => "candidate_available",
        Cancelled => "cancelled",
        Failed => "failed",
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_round_trips_through_its_text() {
        for state in RecordingState::ALL {
            assert_eq!(RecordingState::parse(state.as_str()), Some(*state));
        }
        for state in ProcessingState::ALL {
            assert_eq!(ProcessingState::parse(state.as_str()), Some(*state));
        }
        for state in MinutesState::ALL {
            assert_eq!(MinutesState::parse(state.as_str()), Some(*state));
        }
        assert_eq!(RecordingState::parse("paused"), None);
    }

    #[test]
    fn recording_follows_the_planned_flow() {
        use RecordingState::{Finalising, Ready, Recorded, Recording, Setup};
        let flow = [Setup, Ready, Recording, Finalising, Recorded];
        for pair in flow.windows(2) {
            assert!(pair[0].can_become(pair[1]), "{} -> {}", pair[0], pair[1]);
        }
    }

    #[test]
    fn finished_recordings_do_not_change_state() {
        for end in [RecordingState::Recorded, RecordingState::Failed] {
            for next in RecordingState::ALL {
                assert!(!end.can_become(*next), "{end} -> {next}");
            }
        }
    }

    #[test]
    fn no_state_returns_to_recording() {
        // There is no pause and no resume.
        for state in RecordingState::ALL {
            let allowed = *state == RecordingState::Ready;
            assert_eq!(
                state.can_become(RecordingState::Recording),
                allowed,
                "{state}"
            );
        }
    }

    #[test]
    fn only_a_recording_with_audio_on_disk_becomes_interrupted_and_that_is_recoverable() {
        use RecordingState::{Finalising, Interrupted, Recording};
        for state in RecordingState::ALL {
            assert_eq!(
                state.can_become(Interrupted),
                matches!(state, Recording | Finalising),
                "{state}"
            );
        }
        assert!(Interrupted.can_become(RecordingState::Recorded));
        assert!(Interrupted.can_become(RecordingState::Failed));
        // A start that never reached capture goes back to setup: nothing was recorded.
        assert!(RecordingState::Ready.can_become(RecordingState::Setup));
    }

    #[test]
    fn a_recording_is_recorded_only_when_every_track_left_a_valid_file() {
        use TrackFinalisation::{Failed, Finalised, Pending, Recovered};
        let outcome = RecordingState::after_finalisation;
        assert_eq!(outcome(&[Finalised, Finalised]), RecordingState::Recorded);
        assert_eq!(outcome(&[Finalised, Recovered]), RecordingState::Recorded);
        assert_eq!(outcome(&[Finalised, Failed]), RecordingState::Failed);
        assert_eq!(outcome(&[Pending, Finalised]), RecordingState::Failed);
        assert_eq!(outcome(&[]), RecordingState::Failed);
    }

    #[test]
    fn track_states_round_trip_through_their_text() {
        for state in TrackSource::ALL {
            assert_eq!(TrackSource::parse(state.as_str()), Some(*state));
        }
        for state in TrackFinalisation::ALL {
            assert_eq!(TrackFinalisation::parse(state.as_str()), Some(*state));
        }
        for state in TrackHealth::ALL {
            assert_eq!(TrackHealth::parse(state.as_str()), Some(*state));
        }
    }

    #[test]
    fn the_agenda_is_editable_only_in_setup() {
        for state in RecordingState::ALL {
            assert_eq!(
                state.allows_preparation(),
                *state == RecordingState::Setup,
                "{state}"
            );
        }
    }
}
