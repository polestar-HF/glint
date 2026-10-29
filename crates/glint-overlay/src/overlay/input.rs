//! Platform-independent interaction state machine for the selection overlay.
//!
//! The Win32 layer feeds it pointer and key events in virtual-desktop physical pixels.
//! It answers with side effects to perform (mouse capture, cursor moves, finishing), and
//! [`Interaction::view`] says what each monitor should show. Both are pure, so the whole
//! gesture logic is unit-tested on any OS.

use super::snap;
use crate::geom::{Point, Rect};
use crate::Action;

/// A selection smaller than this in either dimension counts as a click.
pub const MIN_SELECTION: i32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Escape,
    Enter,
    /// Ctrl+C.
    Copy,
    /// Arrow key: move the cursor by this many pixels.
    Nudge(i32, i32),
}

/// Maps a Win32 virtual-key code to an overlay key. `alt_f4` is true for the
/// `WM_SYSKEYDOWN` variant, where F4 means Alt+F4. `repeat` is the "key was already
/// down" bit: Enter and Ctrl+C ignore it, so a key still held from the capture hotkey
/// cannot complete the overlay the moment it appears.
pub fn map_key(vk: u16, ctrl: bool, alt_f4: bool, repeat: bool) -> Option<Key> {
    const VK_RETURN: u16 = 0x0D;
    const VK_ESCAPE: u16 = 0x1B;
    const VK_LEFT: u16 = 0x25;
    const VK_UP: u16 = 0x26;
    const VK_RIGHT: u16 = 0x27;
    const VK_DOWN: u16 = 0x28;
    const VK_C: u16 = 0x43;
    const VK_F4: u16 = 0x73;
    match vk {
        VK_ESCAPE => Some(Key::Escape),
        VK_F4 if alt_f4 => Some(Key::Escape),
        VK_RETURN if !repeat => Some(Key::Enter),
        VK_LEFT => Some(Key::Nudge(-1, 0)),
        VK_RIGHT => Some(Key::Nudge(1, 0)),
        VK_UP => Some(Key::Nudge(0, -1)),
        VK_DOWN => Some(Key::Nudge(0, 1)),
        VK_C if ctrl && !repeat => Some(Key::Copy),
        _ => None,
    }
}

/// Splits a mouse-message `LPARAM` into signed client coordinates. They are negative
/// when the mouse is captured and left the window to the left or top.
pub fn split_lparam(lparam: isize) -> (i32, i32) {
    let x = (lparam & 0xFFFF) as u16 as i16 as i32;
    let y = ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    Cancel,
    /// `rect` is in virtual-desktop pixels and lies inside monitor `monitor`.
    Select { rect: Rect, monitor: usize, action: Action },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    Set,
    Release,
}

/// Side effects the platform layer must perform after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Response {
    pub capture: Option<Capture>,
    pub move_cursor: Option<Point>,
    pub finish: Option<Finish>,
}

impl Response {
    fn finish(f: Finish) -> Self {
        Self { finish: Some(f), ..Self::default() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Highlight {
    #[default]
    None,
    /// Window under the cursor (no drag yet), clipped to the monitor.
    Snap(Rect),
    /// Rectangle being dragged.
    Selection(Rect),
}

impl Highlight {
    pub fn rect(self) -> Option<Rect> {
        match self {
            Highlight::None => None,
            Highlight::Snap(r) | Highlight::Selection(r) => Some(r),
        }
    }

    fn offset(self, dx: i32, dy: i32) -> Self {
        match self {
            Highlight::None => Highlight::None,
            Highlight::Snap(r) => Highlight::Snap(r.offset(dx, dy)),
            Highlight::Selection(r) => Highlight::Selection(r.offset(dx, dy)),
        }
    }
}

/// What one monitor shows. Compared between events to decide which windows to repaint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct View {
    pub highlight: Highlight,
    /// Pixel under the magnifier crosshair, if the magnifier is shown on this monitor.
    pub magnifier: Option<Point>,
}

impl View {
    /// The same view in coordinates relative to `origin` (a monitor's top-left).
    pub fn to_local(self, origin: Point) -> View {
        View {
            highlight: self.highlight.offset(-origin.x, -origin.y),
            magnifier: self.magnifier.map(|p| p.offset(-origin.x, -origin.y)),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Press {
    start: Point,
    monitor: usize,
    /// Window rect (clipped to `monitor`) under the press point; a click selects it.
    snapped: Option<Rect>,
}

pub struct Interaction {
    monitors: Vec<Rect>,
    /// Snap targets, top-most first.
    targets: Vec<Rect>,
    default_action: Action,
    magnifier: bool,
    cursor: Option<Point>,
    press: Option<Press>,
}

impl Interaction {
    pub fn new(monitors: Vec<Rect>, targets: Vec<Rect>, default_action: Action, magnifier: bool) -> Self {
        Self { monitors, targets, default_action, magnifier, cursor: None, press: None }
    }

    pub fn monitor_at(&self, p: Point) -> Option<usize> {
        self.monitors.iter().position(|m| m.contains(p))
    }

    pub fn cursor(&self) -> Option<Point> {
        self.cursor
    }

    /// The snap target under `p`, clipped to the monitor containing `p`.
    fn snap_at(&self, p: Point) -> Option<(Rect, usize)> {
        let m = self.monitor_at(p)?;
        let r = snap::hit_test(&self.targets, p)?.intersect(self.monitors[m])?;
        Some((r, m))
    }

    /// Drag rectangle for the current gesture, clipped to the starting monitor.
    fn drag_rect(&self, press: &Press, to: Point) -> Rect {
        let to = self.monitors.get(press.monitor).map_or(to, |m| m.clamp_point(to));
        Rect::spanning(press.start, to)
    }

    fn is_selection(r: Rect) -> bool {
        r.width() >= MIN_SELECTION && r.height() >= MIN_SELECTION
    }

    /// What releasing (or confirming) right now would select during a press.
    fn press_target(&self, press: &Press) -> Option<Rect> {
        let cursor = self.cursor.unwrap_or(press.start);
        let r = self.drag_rect(press, cursor);
        if Self::is_selection(r) {
            Some(r)
        } else {
            press.snapped
        }
    }

    /// What Enter or Ctrl+C would select right now.
    fn confirm_target(&self) -> Option<(Rect, usize)> {
        match &self.press {
            Some(press) => self.press_target(press).map(|r| (r, press.monitor)),
            None => self.cursor.and_then(|c| self.snap_at(c)),
        }
    }

    pub fn pointer_moved(&mut self, p: Point) -> Response {
        self.cursor = Some(p);
        Response::default()
    }

    pub fn left_down(&mut self, p: Point) -> Response {
        self.cursor = Some(p);
        if self.press.is_some() {
            return Response::default();
        }
        let Some(monitor) = self.monitor_at(p) else {
            return Response::default();
        };
        let snapped = self.snap_at(p).map(|(r, _)| r);
        self.press = Some(Press { start: p, monitor, snapped });
        Response { capture: Some(Capture::Set), ..Response::default() }
    }

    pub fn left_up(&mut self, p: Point) -> Response {
        self.cursor = Some(p);
        let Some(press) = self.press.take() else {
            return Response::default();
        };
        let finish = self.press_target(&press).map(|rect| Finish::Select {
            rect,
            monitor: press.monitor,
            action: self.default_action,
        });
        Response { capture: Some(Capture::Release), move_cursor: None, finish }
    }

    /// Right-click resets a drag in progress, and otherwise cancels the overlay.
    pub fn right_down(&mut self, p: Point) -> Response {
        self.cursor = Some(p);
        if self.press.take().is_some() {
            Response { capture: Some(Capture::Release), ..Response::default() }
        } else {
            Response::finish(Finish::Cancel)
        }
    }

    /// Mouse capture was taken away (another window grabbed it, Alt+Tab, ...).
    pub fn capture_lost(&mut self) -> Response {
        self.press = None;
        Response::default()
    }

    pub fn key(&mut self, key: Key) -> Response {
        match key {
            Key::Escape => Response::finish(Finish::Cancel),
            Key::Enter | Key::Copy => {
                let action = if key == Key::Copy { Action::CopyText } else { self.default_action };
                match self.confirm_target() {
                    Some((rect, monitor)) => {
                        let capture = self.press.take().map(|_| Capture::Release);
                        Response { capture, move_cursor: None, finish: Some(Finish::Select { rect, monitor, action }) }
                    }
                    None => Response::default(),
                }
            }
            Key::Nudge(dx, dy) => match self.cursor {
                Some(c) => Response { move_cursor: Some(c.offset(dx, dy)), ..Response::default() },
                None => Response::default(),
            },
        }
    }

    /// What monitor `index` shows, in virtual-desktop coordinates.
    pub fn view(&self, index: usize) -> View {
        let Some(&monitor) = self.monitors.get(index) else {
            return View::default();
        };
        match (&self.press, self.cursor) {
            (Some(press), cursor) => {
                if press.monitor != index {
                    return View::default();
                }
                let cursor = monitor.clamp_point(cursor.unwrap_or(press.start));
                let r = self.drag_rect(press, cursor);
                let highlight = if Self::is_selection(r) {
                    Highlight::Selection(r)
                } else {
                    press.snapped.map_or(Highlight::None, Highlight::Snap)
                };
                View { highlight, magnifier: self.magnifier.then_some(cursor) }
            }
            (None, Some(cursor)) if monitor.contains(cursor) => View {
                highlight: self.snap_at(cursor).map_or(Highlight::None, |(r, _)| Highlight::Snap(r)),
                magnifier: self.magnifier.then_some(cursor),
            },
            (None, _) => View::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: Rect = Rect::new(-1920, 0, 0, 1080);
    const MAIN: Rect = Rect::new(0, 0, 2560, 1440);

    fn setup() -> Interaction {
        let targets = vec![
            Rect::new(100, 100, 500, 400),      // top-most window
            Rect::new(-300, 50, 300, 600),      // straddles both monitors
            Rect::new(-1920, 0, 2560, 1440),    // desktop
        ];
        Interaction::new(vec![LEFT, MAIN], targets, Action::Translate, true)
    }

    fn selected(r: Response) -> Option<(Rect, usize, Action)> {
        match r.finish {
            Some(Finish::Select { rect, monitor, action }) => Some((rect, monitor, action)),
            _ => None,
        }
    }

    #[test]
    fn hover_snaps_to_topmost_window_clipped_to_monitor() {
        let mut ix = setup();
        ix.pointer_moved(Point::new(200, 200));
        assert_eq!(ix.view(1).highlight, Highlight::Snap(Rect::new(100, 100, 500, 400)));
        assert_eq!(ix.view(1).magnifier, Some(Point::new(200, 200)));
        assert_eq!(ix.view(0), View::default(), "other monitor shows nothing");

        ix.pointer_moved(Point::new(50, 500));
        assert_eq!(ix.view(1).highlight, Highlight::Snap(Rect::new(0, 50, 300, 600)));
        ix.pointer_moved(Point::new(-50, 500));
        assert_eq!(ix.view(0).highlight, Highlight::Snap(Rect::new(-300, 50, 0, 600)));
        assert_eq!(ix.view(1), View::default());
    }

    #[test]
    fn click_selects_snapped_window() {
        let mut ix = setup();
        ix.pointer_moved(Point::new(200, 200));
        assert_eq!(ix.left_down(Point::new(200, 200)).capture, Some(Capture::Set));
        let r = ix.left_up(Point::new(202, 201)); // jitter below 4 px
        assert_eq!(r.capture, Some(Capture::Release));
        assert_eq!(selected(r), Some((Rect::new(100, 100, 500, 400), 1, Action::Translate)));
    }

    #[test]
    fn drag_selects_rectangle_inclusive() {
        let mut ix = setup();
        ix.left_down(Point::new(1000, 800));
        ix.pointer_moved(Point::new(1010, 805));
        assert_eq!(ix.view(1).highlight, Highlight::Selection(Rect::new(1000, 800, 1011, 806)));
        let r = ix.left_up(Point::new(1199, 899));
        assert_eq!(selected(r), Some((Rect::new(1000, 800, 1200, 900), 1, Action::Translate)));
    }

    #[test]
    fn small_drag_counts_as_click_and_shows_snap() {
        let mut ix = setup();
        ix.left_down(Point::new(1000, 800));
        ix.pointer_moved(Point::new(1100, 802)); // 101 × 3: too thin
        assert_eq!(ix.view(1).highlight, Highlight::Snap(MAIN));
        let r = ix.left_up(Point::new(1100, 802));
        assert_eq!(selected(r).map(|s| s.0), Some(MAIN));
    }

    #[test]
    fn small_drag_without_snap_target_does_nothing() {
        let mut ix = Interaction::new(vec![MAIN], Vec::new(), Action::OcrOnly, false);
        ix.left_down(Point::new(10, 10));
        let r = ix.left_up(Point::new(11, 11));
        assert_eq!(r.finish, None);
        assert_eq!(r.capture, Some(Capture::Release));
        assert_eq!(ix.view(0), View::default());
    }

    #[test]
    fn drag_across_monitors_is_clipped_to_start_monitor() {
        let mut ix = setup();
        ix.left_down(Point::new(-100, 100));
        ix.pointer_moved(Point::new(400, 300));
        let v = ix.view(0);
        assert_eq!(v.highlight, Highlight::Selection(Rect::new(-100, 100, 0, 301)));
        assert_eq!(v.magnifier, Some(Point::new(-1, 300)), "magnifier follows the clamped point");
        assert_eq!(ix.view(1), View::default());
        let r = ix.left_up(Point::new(400, 300));
        assert_eq!(selected(r), Some((Rect::new(-100, 100, 0, 301), 0, Action::Translate)));
    }

    #[test]
    fn full_monitor_drag_reaches_last_pixel() {
        let mut ix = setup();
        ix.left_down(Point::new(0, 0));
        let r = ix.left_up(Point::new(2559, 1439));
        assert_eq!(selected(r).map(|s| s.0), Some(MAIN));
    }

    #[test]
    fn right_click_resets_drag_then_cancels() {
        let mut ix = setup();
        ix.left_down(Point::new(10, 10));
        ix.pointer_moved(Point::new(300, 300));
        let r = ix.right_down(Point::new(300, 300));
        assert_eq!(r, Response { capture: Some(Capture::Release), ..Response::default() });
        assert!(matches!(ix.view(1).highlight, Highlight::Snap(_)), "back to hover");
        // The left-up that follows must not select anything.
        assert_eq!(ix.left_up(Point::new(300, 300)).finish, None);
        assert_eq!(ix.right_down(Point::new(300, 300)).finish, Some(Finish::Cancel));
    }

    #[test]
    fn escape_cancels_anytime() {
        let mut ix = setup();
        assert_eq!(ix.key(Key::Escape).finish, Some(Finish::Cancel));
        ix.left_down(Point::new(10, 10));
        assert_eq!(ix.key(Key::Escape).finish, Some(Finish::Cancel));
    }

    #[test]
    fn enter_confirms_snapped_window() {
        let mut ix = setup();
        assert_eq!(ix.key(Key::Enter).finish, None, "no cursor yet");
        ix.pointer_moved(Point::new(200, 200));
        let r = ix.key(Key::Enter);
        assert_eq!(selected(r), Some((Rect::new(100, 100, 500, 400), 1, Action::Translate)));
        assert_eq!(r.capture, None);
    }

    #[test]
    fn ctrl_c_copies_snapped_window_or_selection() {
        let mut ix = setup();
        ix.pointer_moved(Point::new(200, 200));
        assert_eq!(selected(ix.key(Key::Copy)).map(|s| s.2), Some(Action::CopyText));

        ix.left_down(Point::new(1000, 1000));
        ix.pointer_moved(Point::new(1100, 1100));
        let r = ix.key(Key::Copy);
        assert_eq!(r.capture, Some(Capture::Release));
        assert_eq!(selected(r), Some((Rect::new(1000, 1000, 1101, 1101), 1, Action::CopyText)));
    }

    #[test]
    fn ctrl_c_without_target_does_nothing() {
        let mut ix = Interaction::new(vec![MAIN], Vec::new(), Action::Translate, true);
        ix.pointer_moved(Point::new(5, 5));
        assert_eq!(ix.key(Key::Copy), Response::default());
    }

    #[test]
    fn arrows_nudge_cursor() {
        let mut ix = setup();
        assert_eq!(ix.key(Key::Nudge(1, 0)).move_cursor, None);
        ix.pointer_moved(Point::new(10, 10));
        assert_eq!(ix.key(Key::Nudge(-1, 0)).move_cursor, Some(Point::new(9, 10)));
        assert_eq!(ix.key(Key::Nudge(0, 1)).move_cursor, Some(Point::new(10, 11)));
    }

    #[test]
    fn capture_loss_resets_press() {
        let mut ix = setup();
        ix.left_down(Point::new(10, 10));
        ix.capture_lost();
        assert_eq!(ix.left_up(Point::new(500, 500)).finish, None);
    }

    #[test]
    fn magnifier_can_be_disabled_and_snap_needs_targets() {
        let mut ix = Interaction::new(vec![MAIN], Vec::new(), Action::Translate, false);
        ix.pointer_moved(Point::new(10, 10));
        assert_eq!(ix.view(0), View::default());
        assert_eq!(ix.view(7), View::default(), "out-of-range monitor");
    }

    #[test]
    fn view_to_local() {
        let v = View { highlight: Highlight::Selection(Rect::new(-100, 10, -50, 20)), magnifier: Some(Point::new(-60, 15)) };
        let l = v.to_local(Point::new(-1920, 0));
        assert_eq!(l.highlight, Highlight::Selection(Rect::new(1820, 10, 1870, 20)));
        assert_eq!(l.magnifier, Some(Point::new(1860, 15)));
    }

    #[test]
    fn key_mapping() {
        assert_eq!(map_key(0x1B, false, false, false), Some(Key::Escape));
        assert_eq!(map_key(0x0D, false, false, false), Some(Key::Enter));
        assert_eq!(map_key(0x43, true, false, false), Some(Key::Copy));
        assert_eq!(map_key(0x43, false, false, false), None);
        assert_eq!(map_key(0x25, false, false, false), Some(Key::Nudge(-1, 0)));
        assert_eq!(map_key(0x28, false, false, false), Some(Key::Nudge(0, 1)));
        assert_eq!(map_key(0x73, false, true, false), Some(Key::Escape));
        assert_eq!(map_key(0x73, false, false, false), None);
    }

    #[test]
    fn held_keys_do_not_confirm() {
        assert_eq!(map_key(0x0D, false, false, true), None);
        assert_eq!(map_key(0x43, true, false, true), None);
        assert_eq!(map_key(0x1B, false, false, true), Some(Key::Escape));
        assert_eq!(map_key(0x27, false, false, true), Some(Key::Nudge(1, 0)), "arrows auto-repeat");
    }

    #[test]
    fn lparam_split_is_signed() {
        assert_eq!(split_lparam(0x0014_000A), (10, 20));
        let neg = ((-5i16 as u16 as isize) << 16) | (-3i16 as u16 as isize);
        assert_eq!(split_lparam(neg), (-3, -5));
    }
}
