//! When the UI paints.
//!
//! Three ideas, and keeping them apart is what keeps the loop cheap:
//!
//! - [`Dirty`]: something changed what is on screen, so a frame is owed. Every
//!   poller in `App::tick` reports it, and every event the loop handles implies
//!   it.
//! - [`Watch`]: how a poller notices a background thread writing to a shared
//!   slot, the one kind of change nothing else announces.
//! - [`Cadence`]: how soon the loop has to come back when no event will wake
//!   it, and whether the clock alone owes a frame when it does.
//!
//! Two rules keep them honest, and breaking either one looks the same from the
//! outside: a screen that will not update, or a laptop fan that will not stop.
//!
//! **`view` stays a pure render.** A poller hidden inside a widget's `view`
//! only runs when a frame happens, so the only way to keep it fed is to claim
//! the screen is animating, which pins the loop at full frame rate. Same story
//! for a shared slot read mid render, like the model list or the provider
//! quota: it puts data on screen that no poller ever saw. Put a [`Watch`] on
//! the slot, poll it in `App::tick`, render from the [`Watch`].
//!
//! **Whatever draws the motion reports the [`Cadence`]**, next to the span,
//! instead of some condition in `app` that slowly stops meaning what it used
//! to. It describes pixels, not work: a running tool that draws one spinner
//! glyph is [`Cadence::SPINNER`], and a reveal collapsed out of view moves
//! nothing at all.

use std::ops::{BitOr, BitOrAssign};
use std::sync::Arc;
use std::time::Duration;

use crate::animation::SPINNER_FRAME;

/// How long the loop may sleep when nothing is animating. Real events wake it
/// immediately, so this only bounds how stale a polled result can get.
pub const IDLE_POLL: Duration = Duration::from_millis(100);

/// Fast enough that per-character reveals and colour fades look continuous.
const SMOOTH_FRAME: Duration = Duration::from_millis(16);

/// One row of a tool result at 12 rows/s. Retimed by nothing: the reveal keeps
/// its pace whatever `ui.fps` is set to.
pub const REVEAL_FRAME: Duration = Duration::from_nanos(1_000_000_000 / 12);

/// What every poller in the crate is tested against, so the two invariants
/// read the same wherever they are checked.
#[cfg(test)]
pub(crate) mod expect {
    pub(crate) const OWED: &str = "the change must owe a frame or the screen stays stale";
    pub(crate) const QUIET: &str = "nothing changed, so no frame may be owed";
}

/// Something changed what is on screen, so a frame is owed.
///
/// `#[must_use]` is the whole point of the newtype: a poller added to
/// `App::tick` a year from now cannot quietly skip its repaint, because
/// dropping this warns and the workspace builds with `-D warnings`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[must_use]
pub struct Dirty(bool);

impl Dirty {
    pub const NO: Self = Self(false);
    pub const YES: Self = Self(true);

    /// Runs every element. Pollers have side effects, so short-circuiting
    /// would stop draining channels as soon as one reported a change.
    pub fn any(flags: impl IntoIterator<Item = Self>) -> Self {
        flags.into_iter().fold(Self::NO, Self::bitor)
    }

    /// Clears the flag as it reports, so the debt is paid exactly once.
    pub fn take(&mut self) -> bool {
        std::mem::take(self).0
    }
}

impl From<bool> for Dirty {
    fn from(changed: bool) -> Self {
        Self(changed)
    }
}

impl BitOr for Dirty {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Dirty {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}

/// The last value seen in a shared slot a background thread publishes to, so
/// `view` renders from here and never loads the slot itself.
///
/// Every publish stores a fresh `Arc`, and holding the last one alive is what
/// stops its address being reused, so comparing pointers is exact. That asks
/// nothing of the slot: no `PartialEq` (`ProviderUsage` has none), no
/// generation to remember to bump, and no stand-in like a length, which misses
/// a same-sized republish.
pub struct Watch<T>(Option<Arc<T>>);

impl<T> Watch<T> {
    /// Starts on what the slot already holds, so the first poll reports an
    /// arrival and not the watcher's own birth. Only watches that poll from
    /// startup need it; one gated on an open modal is repainted by the opening.
    pub fn seeded(latest: impl Into<Option<Arc<T>>>) -> Self {
        Self(latest.into())
    }

    pub fn poll(&mut self, latest: impl Into<Option<Arc<T>>>) -> Dirty {
        let latest = latest.into();
        if self.0.as_ref().map(Arc::as_ptr) == latest.as_ref().map(Arc::as_ptr) {
            return Dirty::NO;
        }
        self.0 = latest;
        Dirty::YES
    }

    pub fn get(&self) -> Option<&T> {
        self.0.as_deref()
    }
}

impl<T> Default for Watch<T> {
    fn default() -> Self {
        Self(None)
    }
}

/// How soon the loop has to come back when nothing is due to wake it.
///
/// Four sources with different rules. Pixels that move on their own
/// ([`Cadence::SPINNER`], [`Cadence::SMOOTH`], [`Cadence::REVEAL`]) owe a frame
/// once the time is up. A worker mid-answer ([`Cadence::PENDING`]) owes none:
/// looking again is the only way to find the result, and the frame follows only
/// if the poll came back [`Dirty`].
///
/// Anything a component notices by itself is a [`Dirty`]. Calling that a
/// cadence pins the loop at full frame rate for as long as it lasts.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cadence {
    /// Typewriter reveals and colour fades. `ui.fps` retimes this one.
    smooth: Option<Duration>,
    /// Spinner glyphs. Moving, but never retimed below its own glyph frame.
    motion: Option<Duration>,
    /// A worker mid-answer. Polled, never painted on the clock alone.
    poll: Option<Duration>,
    /// Tool rows landing one at a time. Never retimed.
    reveal: Option<Duration>,
}

impl Cadence {
    /// Nothing to come back for. The loop sleeps until something happens.
    pub const IDLE: Self = Self {
        smooth: None,
        motion: None,
        poll: None,
        reveal: None,
    };
    /// Spinner glyphs, which advance once per [`SPINNER_FRAME`]. Painting any
    /// faster redraws the same glyph.
    pub const SPINNER: Self = Self {
        smooth: None,
        motion: Some(SPINNER_FRAME),
        poll: None,
        reveal: None,
    };
    /// Typewriter reveals and colour fades, which change every frame.
    pub const SMOOTH: Self = Self {
        smooth: Some(SMOOTH_FRAME),
        motion: None,
        poll: None,
        reveal: None,
    };
    /// A worker mid-answer, looked at again on the smooth frame so a result
    /// landing between two frames is on screen in the next one.
    pub const PENDING: Self = Self {
        smooth: None,
        motion: None,
        poll: Some(SMOOTH_FRAME),
        reveal: None,
    };
    /// A tool result revealing one row at a time.
    pub const REVEAL: Self = Self {
        smooth: None,
        motion: None,
        poll: None,
        reveal: Some(REVEAL_FRAME),
    };

    /// `cadence` while `applies`, else [`Cadence::IDLE`].
    pub fn when(applies: bool, cadence: Self) -> Self {
        if applies { cadence } else { Self::IDLE }
    }

    /// Combines every source: the soonest deadline and whether any of them
    /// moves on the clock alone.
    pub fn any(cadences: impl IntoIterator<Item = Self>) -> Self {
        cadences.into_iter().fold(Self::IDLE, |a, b| Self {
            smooth: soonest(a.smooth, b.smooth),
            motion: soonest(a.motion, b.motion),
            poll: soonest(a.poll, b.poll),
            reveal: soonest(a.reveal, b.reveal),
        })
    }

    /// Applies the configured smooth frame. Spinner glyphs are clamped so a low
    /// fps never makes the loop ask for them faster than it could paint; polls
    /// and the reveal keep their own pace.
    pub fn retime(self, smooth: Duration) -> Self {
        Self {
            smooth: self.smooth.map(|_| smooth),
            motion: self.motion.map(|motion| motion.max(smooth)),
            ..self
        }
    }

    /// How long the loop may sleep, or `None` to sleep until something wakes
    /// it.
    pub fn frame(self) -> Option<Duration> {
        [self.smooth, self.motion, self.poll, self.reveal]
            .into_iter()
            .flatten()
            .min()
    }

    /// Whether the next frame differs from this one on the clock alone.
    pub fn moves(self) -> bool {
        self.smooth.is_some() || self.motion.is_some() || self.reveal.is_some()
    }
}

fn soonest(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Two cadences are equal when they ask for the same next frame and the same
/// motion; which source supplied the frame is an implementation detail.
impl PartialEq for Cadence {
    fn eq(&self, other: &Self) -> bool {
        self.frame() == other.frame() && self.moves() == other.moves()
    }
}

impl Eq for Cadence {}

#[cfg(test)]
mod tests {
    use super::expect::{OWED, QUIET};
    use super::{Cadence, Dirty, Watch};
    use std::sync::Arc;
    use std::time::Duration;

    const EXPECT_SETTLED: &str = "nothing to combine must leave the loop asleep";
    const EXPECT_SOONEST: &str = "the soonest source must win over the whole list";
    const EXPECT_MOTION_KEPT: &str = "a pending poll must not swallow another source's motion";
    const EXPECT_UNSEEN: &str = "an unpolled watch has nothing to render";

    #[test]
    fn any_runs_every_poller() {
        let mut ran = 0;
        let dirty = Dirty::any(
            [Dirty::YES, Dirty::NO, Dirty::NO]
                .into_iter()
                .inspect(|_| ran += 1),
        );
        assert_eq!(dirty, Dirty::YES);
        assert_eq!(ran, 3, "a reported change must not stop later pollers");
    }

    #[test]
    fn take_pays_the_debt_once() {
        let mut dirty = Dirty::YES;
        assert!(dirty.take());
        assert!(!dirty.take());
    }

    /// The empty fold is the common case: a settled session takes it on every
    /// loop turn.
    #[test]
    fn any_takes_the_soonest_frame_and_keeps_the_motion() {
        assert_eq!(Dirty::any([]), Dirty::NO, "{EXPECT_SETTLED}");
        assert_eq!(Cadence::any([]), Cadence::IDLE, "{EXPECT_SETTLED}");

        let picked = Cadence::any([Cadence::IDLE, Cadence::SMOOTH, Cadence::SPINNER]);
        assert_eq!(picked.frame(), Cadence::SMOOTH.frame(), "{EXPECT_SOONEST}");

        let mixed = Cadence::any([Cadence::SPINNER, Cadence::PENDING]);
        assert_eq!(mixed.frame(), Cadence::PENDING.frame(), "{EXPECT_SOONEST}");
        assert!(mixed.moves(), "{EXPECT_MOTION_KEPT}");
    }

    #[test]
    fn retime_moves_only_the_smooth_source() {
        let faster = Duration::from_millis(4);
        let slower = Duration::from_millis(83);

        assert_eq!(Cadence::SMOOTH.retime(faster).frame(), Some(faster));
        assert_eq!(Cadence::SMOOTH.retime(slower).frame(), Some(slower));

        assert_eq!(
            Cadence::PENDING.retime(faster).frame(),
            Cadence::PENDING.frame()
        );
        assert_eq!(
            Cadence::REVEAL.retime(faster).frame(),
            Cadence::REVEAL.frame()
        );
        assert_eq!(
            Cadence::SPINNER.retime(faster).frame(),
            Cadence::SPINNER.frame()
        );
        assert_eq!(Cadence::SPINNER.retime(slower).frame(), Some(slower));

        assert!(Cadence::REVEAL.retime(faster).moves());
        assert!(!Cadence::PENDING.retime(faster).moves());
    }

    /// A generation counter answers this too, as long as every publisher
    /// remembers to bump one. Identity asks nothing of the publisher, and an
    /// equal value republished is still a change worth a frame.
    #[test]
    fn watch_reports_each_arrival_once() {
        let mut watch = Watch::default();
        assert_eq!(watch.get(), None, "{EXPECT_UNSEEN}");
        assert_eq!(watch.poll(None), Dirty::NO, "{QUIET}");

        let first = Arc::new(7);
        assert_eq!(watch.poll(Arc::clone(&first)), Dirty::YES, "{OWED}");
        assert_eq!(watch.get(), Some(&7));
        assert_eq!(watch.poll(first), Dirty::NO, "{QUIET}");

        assert_eq!(watch.poll(Arc::new(7)), Dirty::YES, "{OWED}");
        assert_eq!(watch.poll(None), Dirty::YES, "{OWED}");
        assert_eq!(watch.get(), None, "{EXPECT_UNSEEN}");
    }
}
