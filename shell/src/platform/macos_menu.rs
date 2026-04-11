use objc2::sel;
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem, NSWindow};
use objc2_foundation::{MainThreadMarker, ns_string};

pub(crate) fn initialize_platform_menu() {
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("Cannot configure macOS menu outside the main thread");
        return;
    };

    // Disable AppKit window tabbing globally so "Show Tab Bar" is not offered.
    NSWindow::setAllowsAutomaticWindowTabbing(false, mtm);

    let app = NSApplication::sharedApplication(mtm);

    let Some(main_menu) = app.mainMenu() else {
        log::warn!("NSApplication has no main menu; skipping fullscreen menu setup");
        return;
    };

    let view_submenu = ensure_view_submenu(&main_menu, mtm);

    if view_submenu
        .itemWithTitle(ns_string!("Enter Full Screen"))
        .is_some()
        || view_submenu
            .itemWithTitle(ns_string!("Toggle Full Screen"))
            .is_some()
    {
        return;
    }

    let fullscreen_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            ns_string!("Enter Full Screen"),
            Some(sel!(toggleFullScreen:)),
            ns_string!("f"),
        )
    };

    fullscreen_item.setKeyEquivalentModifierMask(
        NSEventModifierFlags::Command | NSEventModifierFlags::Control,
    );

    // Allow user-defined App Shortcut overrides to replace key equivalents.
    NSMenuItem::setUsesUserKeyEquivalents(true, mtm);
    view_submenu.addItem(&fullscreen_item);
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
