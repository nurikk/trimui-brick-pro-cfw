# Artbook navigation contract

`ui-model` is the project-owned, clean-room state contract for the default UI identity **Artbook**. It is a simulator-first model: consumers provide inputs and render the resulting data, while this package performs no I/O.

## Composition

- The logical canvas is 320x240 with a fixed 4:3 aspect-ratio contract.
- Home is a full-screen, controller-first menu for Home, Systems, Games, Search, Favorites, Settings, Game Switcher, and Recovery.
- Systems expose large generated artwork and logo references.
- Games expose an ordered, dense list contract with generated box-art and screenshot references, description, optional rating, optional release date, and favorite state.
- The view contract declares large system artwork, dense game rows, full metadata, and a full-screen menu surface.
- The help strip exposes readable labels for primary, secondary, menu, and start controls. Clock and battery values are supplied as model inputs; the reducer never reads a clock or battery.
- Preferences are typed: artwork mode, metadata visibility, font scale, and color scheme. The default is large artwork, full metadata, standard font, and ink colors.

All artwork references are neutral generated identifiers. They are references only; this package contains no media, fonts, image loading, or theme implementation.

## Controller behavior and transitions

`Action::MoveSelection` moves through the ordered menu. Up/left move backward and down/right move forward, wrapping at the list boundary. Disabled entries remain visible with a typed capability reason and are skipped during movement. `Action::ActivateSelected` activates only the selected enabled entry. `Action::Back` dismisses a modal first; without a modal it returns to Home. Every menu contains an explicit selection index, item id, and selected flag. `ConfirmModal` dismisses and executes the command only for `ModalState::Confirm`; informational and unavailable modals are simply dismissed.

Home activates the primary browsing routes. Systems activate a typed system selection and open Games filtered to that system. Games, Favorites, and Search expose ordered generated game rows. Search stores a bounded query in state and rebuilds the ordered matching menu. Favorite changes are explicit `ToggleFavorite` actions and become model feedback; they do not call a service. Launch is an explicit session request and is unavailable when the session capability is false.

Settings actions replace one typed preference at a time. Settings persistence is a capability: when unavailable, the preference remains unchanged and the reducer emits an `Unavailable` modal. Persistence keys are typed and versioned; the preferences fixture shows the public JSON shape. The four current Artbook preference entries are generated fixture data only, not the settings architecture. Production `Route::Settings` must consume the declarative settings/menu projection owned by `t_45dfd680`/`t_bf010be5` and must not require hand-coded menus. This card does not integrate those sibling crates.

## Modal, error, and recovery behavior

Capability failures use `ModalState::Unavailable` with a typed capability, stable error code, and bounded message. Modal dismissal and confirmation clear only the modal. A disabled menu entry never silently performs its command. Recovery is a normal route with a safe return-to-Home entry.

The initial splash is a generated Artbook placeholder reference. `FinishSplash` changes it to `Ready`. `ShowFallback` changes the route to Recovery and installs a generated fallback reference with a typed reason. Fallback state does not attempt to load media or retry an unavailable operation.

## Reserved typed contracts

Scraper state is reserved for scraper settings, per-game requests, bulk queue and progress, ambiguous choices, deterministic candidate selection, pause, resume, cancel, complete, and non-blocking status errors. Candidate selection accepts an index into the existing ordered choice list and stores the chosen identifier only when that index exists. Its actions only change typed state; they do not scrape, fetch, parse, or process media.

Wi-Fi state is reserved for scan, access-point selection, hidden/manual SSID flow, masked-password keyboard request, connect, disconnect, forget, retry, cancel, progress, unavailable, and error states. `EnterSsid` records a bounded manual or hidden SSID intent and transitions to typed password-entry state; the subsequent keyboard request remains masked. The contract contains no password field and performs no radio or credential operation.

## Determinism and boundaries

`reduce(&UiState, Action) -> UiState` clones the supplied state and consumes all action data explicitly. It has no clock, random, environment, filesystem, process, renderer, emulator, network, hardware, or external-service dependency. Ordered vectors are used wherever serialized ordering is public. Repeated serialization of an unchanged state is byte-identical by test.

Ports are narrow interfaces for catalog, settings, favorites, session, and platform capabilities. Only generated fake providers are included. Integrations can implement those ports outside this package without changing the reducer contract.

This package is not a renderer, UI toolkit, theme engine, catalog, scraper, radio implementation, emulator integration, device abstraction, packaging component, or simulator control plane. It intentionally contains no actual artwork, fonts, images, scraped metadata, media payloads, or service implementation.
