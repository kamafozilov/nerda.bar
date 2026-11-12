//! The menu bar items, one per provider: the provider's mark and a percentage. A click opens
//! the panel under the item; a secondary click opens a menu. Rune has no macOS status item
//! yet, so this is AppKit.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSBezierPath, NSCellImagePosition, NSColor, NSControlStateValueOff, NSControlStateValueOn, NSEventMask, NSEventModifierFlags,
    NSEventType, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSImage, NSMenu, NSMenuDelegate, NSMenuItem, NSScreen, NSStatusBar,
    NSStatusItem, NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSAttributedString, NSDictionary, NSError, NSPoint, NSRect, NSSize, NSString, NSURL};
use rune::AppHandle;

use crate::model::Severity;
use crate::ui::{self, Anchor, Ui};

const TAG_USED: isize = 1;
const TAG_LOGIN: isize = 2;
/// The mark's side in points: the size of the system's own menu bar icons.
const MARK: f64 = 16.0;

/// A provider's mark as an SVG path, and the side of the square it is drawn in.
pub struct Mark {
    pub path: &'static str,
    pub view: f64,
}

pub const CLAUDE_MARK: Mark = Mark { path: include_str!("../assets/providers/claude.path"), view: 100.0 };

struct Item {
    id: &'static str,
    status: Retained<NSStatusItem>,
    menu: Retained<NSMenu>,
    /// What the item shows now, so an unchanged refresh draws nothing.
    shown: Option<(String, Severity, bool)>,
}

thread_local! {
    static ITEMS: RefCell<Vec<Item>> = const { RefCell::new(Vec::new()) };
    /// Whether percentages show what is used; the menu's checkmark reads it.
    static SHOW_USED: Cell<bool> = const { Cell::new(false) };
}

struct Ivars {
    app: AppHandle<Ui>,
    data_dir: PathBuf,
    provider: &'static str,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the target is only used on the
    // main thread, where AppKit sends actions.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "NerdaBarItemTarget"]
    #[ivars = Ivars]
    struct Target;

    impl Target {
        #[unsafe(method(clicked:))]
        fn clicked(&self, _sender: Option<&AnyObject>) {
            let mtm = self.mtm();
            let event = NSApplication::sharedApplication(mtm).currentEvent();
            let secondary = event.is_some_and(|event| event.r#type() == NSEventType::RightMouseUp || event.modifierFlags().contains(NSEventModifierFlags::Control));
            // The handles leave the registry before AppKit runs anything: a menu tracks in its
            // own run loop, where a refresh may want the registry.
            let found = ITEMS.with_borrow(|items| items.iter().find(|item| item.id == self.ivars().provider).map(|item| (item.status.clone(), item.menu.clone())));
            let Some((status, menu)) = found else { return };
            if secondary {
                // An item with a menu can't also take clicks, so the menu is lent for one click.
                status.setMenu(Some(&menu));
                if let Some(button) = status.button(mtm) {
                    // SAFETY: a click with no sender, on the main thread.
                    unsafe { button.performClick(None) };
                }
                status.setMenu(None);
            } else if let Some(anchor) = anchor(&status, mtm) {
                self.ivars().app.update(move |s, tx| ui::toggle(s, anchor, tx));
            }
        }

        #[unsafe(method(refresh:))]
        fn refresh(&self, _sender: Option<&AnyObject>) {
            self.ivars().app.update(|s, _| ui::refresh(s));
        }

        #[unsafe(method(toggleUsed:))]
        fn toggle_used(&self, _sender: Option<&AnyObject>) {
            let show_used = !SHOW_USED.get();
            self.ivars().app.update(move |s, _| ui::set_show_used(s, show_used));
        }

        #[unsafe(method(toggleLogin:))]
        fn toggle_login(&self, _sender: Option<&AnyObject>) {
            if let Err(reason) = login_item::set(!login_item::enabled()) {
                eprintln!("nerda-bar: launch at login: {reason}");
            }
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: Option<&AnyObject>) {
            open_path(&self.ivars().data_dir.join("config.json"));
        }

        #[unsafe(method(showData:))]
        fn show_data(&self, _sender: Option<&AnyObject>) {
            open_path(&self.ivars().data_dir);
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            self.ivars().app.quit();
        }
    }

    unsafe impl NSObjectProtocol for Target {}

    unsafe impl NSMenuDelegate for Target {
        /// Checkmarks follow the real state each time the menu opens: Launch at Login can
        /// also be switched off in System Settings.
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_needs_update(&self, menu: &NSMenu) {
            let check = |tag: isize, on: bool| {
                if let Some(item) = menu.itemWithTag(tag) {
                    item.setState(if on { NSControlStateValueOn } else { NSControlStateValueOff });
                }
            };
            check(TAG_USED, SHOW_USED.get());
            check(TAG_LOGIN, login_item::enabled());
        }
    }
);

fn open_path(path: &std::path::Path) {
    let path = NSString::from_str(&path.to_string_lossy());
    NSWorkspace::sharedWorkspace().openURL(&NSURL::fileURLWithPath(&path));
}

/// Where the item is on screen, for the panel to open under it.
fn anchor(status: &NSStatusItem, mtm: MainThreadMarker) -> Option<Anchor> {
    let window = status.button(mtm)?.window()?;
    let frame = window.frame();
    // AppKit measures up from the primary display's bottom-left corner; the panel is placed
    // down from its top-left.
    let primary = NSScreen::screens(mtm).firstObject()?.frame();
    let screen = window.screen().map_or(primary, |screen| screen.frame());
    Some(Anchor {
        x: (frame.origin.x + frame.size.width / 2.0) as f32,
        y: (primary.size.height - frame.origin.y) as f32,
        screen: [screen.origin.x as f32, (screen.origin.x + screen.size.width) as f32],
    })
}

/// The points of an SVG path that uses absolute `M L H V C Z`, scaled by `scale`.
#[derive(Debug, PartialEq)]
enum Segment {
    Move([f64; 2]),
    Line([f64; 2]),
    Curve([f64; 2], [f64; 2], [f64; 2]),
    Close,
}

fn parse_path(path: &str, scale: f64) -> Option<Vec<Segment>> {
    let bytes = path.as_bytes();
    let mut at = 0;
    let mut segments = Vec::new();
    let mut command = 0u8;
    let mut pen = [0.0f64; 2];
    // The next number: digits with an optional sign, fraction and exponent.
    let number = |at: &mut usize| -> Option<f64> {
        while bytes.get(*at).is_some_and(|b| b.is_ascii_whitespace() || *b == b',') {
            *at += 1;
        }
        let start = *at;
        if bytes.get(*at).is_some_and(|b| *b == b'-' || *b == b'+') {
            *at += 1;
        }
        let mut seen_dot = false;
        while let Some(b) = bytes.get(*at) {
            match b {
                b'0'..=b'9' => *at += 1,
                b'.' if !seen_dot => {
                    seen_dot = true;
                    *at += 1;
                }
                _ => break,
            }
        }
        path.get(start..*at)?.parse::<f64>().ok().map(|value| value * scale)
    };
    loop {
        while bytes.get(at).is_some_and(|b| b.is_ascii_whitespace() || *b == b',') {
            at += 1;
        }
        let Some(&next) = bytes.get(at) else { break };
        if next.is_ascii_alphabetic() {
            command = next;
            at += 1;
            if command == b'Z' || command == b'z' {
                segments.push(Segment::Close);
                continue;
            }
        }
        match command {
            b'M' => {
                pen = [number(&mut at)?, number(&mut at)?];
                segments.push(Segment::Move(pen));
                // Further pairs after a move are lines.
                command = b'L';
            }
            b'L' => {
                pen = [number(&mut at)?, number(&mut at)?];
                segments.push(Segment::Line(pen));
            }
            b'H' => {
                pen[0] = number(&mut at)?;
                segments.push(Segment::Line(pen));
            }
            b'V' => {
                pen[1] = number(&mut at)?;
                segments.push(Segment::Line(pen));
            }
            b'C' => {
                let (c1, c2) = ([number(&mut at)?, number(&mut at)?], [number(&mut at)?, number(&mut at)?]);
                pen = [number(&mut at)?, number(&mut at)?];
                segments.push(Segment::Curve(c1, c2, pen));
            }
            _ => return None,
        }
    }
    Some(segments)
}

/// A provider's mark as a template image, which AppKit tints for the menu bar.
fn mark_image(mark: &Mark, name: &str) -> Retained<NSImage> {
    let segments = parse_path(mark.path, MARK / mark.view).unwrap_or_default();
    let draw = RcBlock::new(move |_rect: NSRect| -> Bool {
        let point = |p: &[f64; 2]| NSPoint::new(p[0], p[1]);
        let path = NSBezierPath::bezierPath();
        for segment in &segments {
            match segment {
                Segment::Move(p) => path.moveToPoint(point(p)),
                Segment::Line(p) => path.lineToPoint(point(p)),
                Segment::Curve(c1, c2, p) => path.curveToPoint_controlPoint1_controlPoint2(point(p), point(c1), point(c2)),
                Segment::Close => path.closePath(),
            }
        }
        NSColor::blackColor().set();
        path.fill();
        Bool::YES
    });
    // Flipped, so the coordinates read top-down as SVG's do.
    let image = NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(MARK, MARK), true, &draw);
    image.setTemplate(true);
    image.setAccessibilityDescription(Some(&NSString::from_str(name)));
    image
}

/// Adds a provider's menu bar item. Call on the main thread, after the app has started.
pub fn install(app: AppHandle<Ui>, data_dir: PathBuf, provider: &'static str, name: &str, mark: &Mark) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let status = NSStatusBar::systemStatusBar().statusItemWithLength(-1.0);
    // Remembered by macOS under this name, so the item keeps the place it is dragged to.
    status.setAutosaveName(Some(&NSString::from_str(&format!("nerda.bar.{provider}"))));

    let target = Target::alloc(mtm).set_ivars(Ivars { app, data_dir, provider });
    // SAFETY: NSObject's `init` on a freshly allocated subclass instance.
    let target: Retained<Target> = unsafe { msg_send![super(target), init] };

    if let Some(button) = status.button(mtm) {
        button.setImage(Some(&mark_image(mark, name)));
        button.setImagePosition(NSCellImagePosition::ImageLeading);
        // SAFETY: the selector is the target's own method, and the target outlives the item.
        unsafe {
            button.setTarget(Some(&target));
            button.setAction(Some(sel!(clicked:)));
        }
        button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
    }

    let item = |title: &str, action: Sel| {
        // SAFETY: the selectors are the target's own methods, and the target outlives the menu.
        let item = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(title), Some(action), &NSString::new()) };
        unsafe { item.setTarget(Some(&target)) };
        item
    };
    let tagged = |item: Retained<NSMenuItem>, tag: isize| {
        item.setTag(tag);
        item
    };

    let menu = NSMenu::new(mtm);
    menu.addItem(&item(&format!("Refresh {name}"), sel!(refresh:)));
    menu.addItem(&tagged(item("Show Used Instead of Left", sel!(toggleUsed:)), TAG_USED));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let login = tagged(item("Launch at Login", sel!(toggleLogin:)), TAG_LOGIN);
    if !login_item::available() {
        // A bare binary has no bundle for macOS to register.
        // SAFETY: clearing an action is always valid; AppKit then shows the item disabled.
        unsafe { login.setAction(None) };
        login.setToolTip(Some(&NSString::from_str("Available when run as Nerda Bar.app")));
    }
    menu.addItem(&login);
    menu.addItem(&item("Open Settings File", sel!(openSettings:)));
    menu.addItem(&item("Show Data Folder", sel!(showData:)));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let quit = item("Quit Nerda Bar", sel!(quit:));
    quit.setKeyEquivalent(&NSString::from_str("q"));
    quit.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
    menu.addItem(&quit);
    menu.setDelegate(Some(ProtocolObject::from_ref(&*target)));

    // The target lives as long as the app: AppKit holds it weakly.
    std::mem::forget(target);
    ITEMS.with_borrow_mut(|items| items.push(Item { id: provider, status, menu, shown: None }));
}

/// Where a provider's item is on screen. Call on the main thread.
pub fn anchor_of(provider: &str) -> Option<Anchor> {
    let mtm = MainThreadMarker::new()?;
    ITEMS.with_borrow(|items| items.iter().find(|item| item.id == provider).and_then(|item| anchor(&item.status, mtm)))
}

/// Remembers which way percentages read, for the menu's checkmark.
pub fn set_show_used(show_used: bool) {
    SHOW_USED.set(show_used);
}

/// Puts `text` beside a provider's mark: coloured when the limit is running low, dimmed when
/// the numbers are not current. Call on the main thread.
pub fn set(provider: &str, text: &str, severity: Severity, stale: bool, tooltip: &str) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    ITEMS.with_borrow_mut(|items| {
        let Some(item) = items.iter_mut().find(|item| item.id == provider) else { return };
        let Some(button) = item.status.button(mtm) else { return };
        button.setToolTip(Some(&NSString::from_str(tooltip)));
        let shown = (text.to_owned(), severity, stale);
        if item.shown.as_ref() == Some(&shown) {
            return;
        }
        // Digits of one width, so the item doesn't shift its neighbours as the number changes.
        let font = NSFont::monospacedDigitSystemFontOfSize_weight(NSFont::systemFontSize() - 1.0, 0.0);
        let colour = match severity {
            Severity::Normal => None,
            Severity::Warning => Some(NSColor::systemOrangeColor()),
            Severity::Critical => Some(NSColor::systemRedColor()),
        };
        // SAFETY: the attribute names are AppKit's constant strings.
        let (font_key, colour_key) = unsafe { (NSFontAttributeName, NSForegroundColorAttributeName) };
        let font: &AnyObject = font.as_ref();
        let attributes = match &colour {
            Some(colour) => NSDictionary::from_slices(&[font_key, colour_key], &[font, colour.as_ref()]),
            // No colour of its own: AppKit draws it as the menu bar's text.
            None => NSDictionary::from_slices(&[font_key], &[font]),
        };
        let title = NSString::from_str(&format!(" {text}"));
        // SAFETY: the attributes are a font and a colour under their own keys.
        let title = unsafe { NSAttributedString::initWithString_attributes(NSAttributedString::alloc(), &title, Some(&attributes)) };
        button.setAttributedTitle(&title);
        button.setAppearsDisabled(stale);
        item.shown = Some(shown);
    });
}

/// Launch at login through `SMAppService.mainApp` (macOS 13 and later).
mod login_item {
    use super::*;

    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    const STATUS_ENABLED: isize = 1;

    fn service() -> Option<Retained<AnyObject>> {
        let class = AnyClass::get(c"SMAppService")?;
        // SAFETY: a class property returning the shared service object.
        unsafe { msg_send![class, mainAppService] }
    }

    /// Whether this process is an app bundle macOS can register.
    pub fn available() -> bool {
        let in_bundle = std::env::current_exe().is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"));
        in_bundle && service().is_some()
    }

    pub fn enabled() -> bool {
        service().is_some_and(|service| {
            // SAFETY: `status` returns the SMAppServiceStatus enum, an NSInteger.
            let status: isize = unsafe { msg_send![&*service, status] };
            status == STATUS_ENABLED
        })
    }

    pub fn set(on: bool) -> Result<(), String> {
        let service = service().ok_or("ServiceManagement is unavailable")?;
        let mut error: *mut NSError = std::ptr::null_mut();
        // SAFETY: both methods take an NSError out-pointer and return BOOL.
        let ok: bool = unsafe {
            if on { msg_send![&*service, registerAndReturnError: &mut error] } else { msg_send![&*service, unregisterAndReturnError: &mut error] }
        };
        if ok {
            return Ok(());
        }
        // SAFETY: on failure the out-pointer holds an autoreleased NSError or stays null.
        Err(unsafe { error.as_ref() }.map_or_else(|| "macOS refused the request".to_owned(), |e| e.localizedDescription().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_parses_into_segments() {
        let segments = parse_path("M10 20L30-5H4.5V6C1 2 3 4 5 6Z", 0.5).unwrap();
        assert_eq!(
            segments,
            [
                Segment::Move([5.0, 10.0]),
                Segment::Line([15.0, -2.5]),
                Segment::Line([2.25, -2.5]),
                Segment::Line([2.25, 3.0]),
                Segment::Curve([0.5, 1.0], [1.5, 2.0], [2.5, 3.0]),
                Segment::Close,
            ]
        );
        assert!(parse_path("M1 2 Q3 4 5 6", 1.0).is_none());
    }

    #[test]
    fn the_claude_mark_parses_whole_and_stays_in_its_square() {
        let segments = parse_path(CLAUDE_MARK.path, 1.0).unwrap();
        assert!(segments.len() > 100);
        assert_eq!(segments.last(), Some(&Segment::Close));
        for segment in &segments {
            let points: &[[f64; 2]] = match segment {
                Segment::Move(p) | Segment::Line(p) => std::slice::from_ref(p),
                Segment::Curve(a, b, c) => &[*a, *b, *c],
                Segment::Close => &[],
            };
            assert!(points.iter().flatten().all(|v| (0.0..=CLAUDE_MARK.view).contains(v)), "{segment:?}");
        }
    }
}
