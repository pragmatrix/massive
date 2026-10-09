# Keys are delivered top-down along the keyboard focus path, and the consumer is the input focus

## Status: accepted

Keys used to take one of two routes: a desktop shortcut matched before input processing and was planned directly, or the event router delivered the key to the keyboard-focused target. Only the second route cleared pointer focus, so shortcuts such as Cmd+arrows left the hover outline under a stationary pointer. The desktop now delivers every key down the keyboard focus path, outermost first. The level that consumes it becomes the **input focus**, and the hover outline and mouse cursor visibility are derived from it.

- **Capture first, no bubbling.** Outer levels get first refusal, which keeps desktop shortcuts ahead of the application as today. Local navigation that passes a key up to a parent was considered and deferred; navigation stays a global desktop-level shortcut for now.
- **The input focus is the consumer, not a mode.** A key consumed by the desktop, a project, or a launcher leaves the user "at the desktop": the hover outline marks the keyboard-focused target. A key that reaches the application moves the input focus into the application and removes the outline. A key that falls off the end of the path with no application (a launcher dropping it) still sets the input focus to the focused leaf, as the old router did by clearing pointer focus on any key.
- **The input focus is stored by the desktop, not the router.** The router is generic and has no knowledge of hierarchy (it hit-tests and tracks pointer focus). Walking the path and validating the input focus against it needs the hierarchy, so both stay in the desktop system. The router keeps only pointer focus, and the desktop resets the input focus whenever pointer focus becomes set again.
- **The desktop is a level with its own presenter.** The root shortcuts (navigation, zoom, start, close) move into a desktop presenter that expands later. Project and launcher presenters implement the same key handler. The launcher decides itself whether a key meant for it is dropped.
- **The instance presenter is the last level and forwards through a change.** It consumes every key that reaches it and returns a change that forwards the key to its view, which the dispatch executes with the instance manager. The walk is uniform: every level returns changes, and no step after the walk hands keys to the application.
- **Behaviour-preserving first step.** Only the keys handled today move into the walk: the root shortcuts, plain Enter on a project, and the launcher's Enter and Cmd+T. The one visible change is that shortcuts now clear the pointer hover like every other key.
- **Keyboard only for now.** Handlers receive the input event with its device state. Pointer events need coordinate transformation along the path and keep their existing route.

## Considered options

- *Run the router first and drop its delivery when a shortcut matched.* Smaller, but it keeps the two routes and the consumption rule outside the levels.
- *A pressed-key map that remembers where each press went and sends the release there.* Rejected for now in favour of delivering releases like presses.
- *Put the walk and the input focus in the router.* Rejected: it would give a hierarchy-free component hierarchy knowledge.

## Consequences

- Releases and repeats walk the chain. A level that consumed a press must also consume its release, but the release context can differ (Cmd may already be up), so a symmetric decision per key is the open risk: a swallowed release would leave a key down in the application, a missed one gives it a release without a press.
- Releases do not change the input focus.
- The launcher sees every key passing through while a deeper target is focused. It must tell from its own state whether the key is meant for it; navigation already focuses a launcher's instance, never a launcher that has one.
