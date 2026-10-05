use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem, NSWindow};
use objc2_foundation::ns_string;
use objc2_foundation::{MainThreadMarker, NSObject, NSObjectProtocol};
use winit::event_loop::EventLoopProxy;

use crate::shell::ShellCommand;

#[derive(Debug)]
struct FullscreenActionIvars {
    proxy: EventLoopProxy<ShellCommand>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = FullscreenActionIvars]
    struct FullscreenAction;

    unsafe impl NSObjectProtocol for FullscreenAction {}

    impl FullscreenAction {
        #[unsafe(method(requestFullscreenToggle:))]
        fn request_fullscreen_toggle(&self, _sender: &NSMenuItem) {
            if let Err(error) = self
                .ivars()
                .proxy
                .send_event(ShellCommand::FullscreenRequested)
            {
                log::error!("Failed to send fullscreen request to the event loop: {error:?}");
            }
        }
    }
);

impl FullscreenAction {
    fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<ShellCommand>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FullscreenActionIvars { proxy });
        // SAFETY: NSObject's init method is valid for this subclass.
        unsafe { msg_send![super(this), init] }
    }
}

pub(crate) fn initialize_platform_menu(proxy: EventLoopProxy<ShellCommand>) {
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("Cannot configure macOS menu outside the main thread");
        return;
    };

    NSWindow::setAllowsAutomaticWindowTabbing(false, mtm);

    let app = NSApplication::sharedApplication(mtm);

    let Some(main_menu) = app.mainMenu() else {
        log::warn!("NSApplication has no main menu; skipping fullscreen menu setup");
        return;
    };

    let view_submenu = ensure_view_submenu(&main_menu, mtm);

    let existing_item = view_submenu
        .itemWithTitle(ns_string!("Enter Full Screen"))
        .or_else(|| view_submenu.itemWithTitle(ns_string!("Toggle Full Screen")));
    let add_item = existing_item.is_none();
    let fullscreen_item = existing_item.unwrap_or_else(|| unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            ns_string!("Enter Full Screen"),
            Some(sel!(requestFullscreenToggle:)),
            ns_string!("f"),
        )
    });
    let action = FullscreenAction::new(mtm, proxy);
    unsafe {
        fullscreen_item.setAction(Some(sel!(requestFullscreenToggle:)));
        fullscreen_item.setTarget(Some(&action));
        fullscreen_item.setRepresentedObject(Some(&action));
    }

    fullscreen_item.setKeyEquivalentModifierMask(
        NSEventModifierFlags::Command | NSEventModifierFlags::Control,
    );

    if add_item {
        view_submenu.addItem(&fullscreen_item);
    }
}

pub(crate) fn toggle_fullscreen() {
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("Cannot toggle macOS fullscreen outside the main thread");
        return;
    };

    let app = NSApplication::sharedApplication(mtm);
    unsafe {
        app.sendAction_to_from(sel!(toggleFullScreen:), None, None);
    }
}

fn ensure_view_submenu(main_menu: &NSMenu, mtm: MainThreadMarker) -> objc2::rc::Retained<NSMenu> {
    let view_title = ns_string!("View");

    if let Some(existing_view_item) = main_menu.itemWithTitle(view_title) {
        if let Some(submenu) = existing_view_item.submenu() {
            return submenu;
        }

        let submenu = NSMenu::new(mtm);
        submenu.setTitle(view_title);
        existing_view_item.setSubmenu(Some(&submenu));
        return submenu;
    }

    let view_item = NSMenuItem::new(mtm);
    view_item.setTitle(view_title);

    let submenu = NSMenu::new(mtm);
    submenu.setTitle(view_title);

    view_item.setSubmenu(Some(&submenu));
    main_menu.addItem(&view_item);

    submenu
}
