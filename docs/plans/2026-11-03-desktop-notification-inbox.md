# Desktop notification inbox

Design review: [hero card and inbox draft](https://p.superdesign.dev/draft/a7088239-5dc3-4dac-b947-8f4c21f02c84).

## Decisions

- The desktop shows the same private notification events and wording as the web app's Notifications page.
- A pixel bell stays beside the walking hero and follows it, including while the stats card is closed. Clicking it opens the inbox directly; clicking the hero continues to open the stats card. Hiding the hero hides the bell. There is no system banner.
- The bell opens a panel in the same overlay, following the existing friend panel's back navigation. It shows the ten newest unread events, plus read events that still have a claimable reward. Rows remain ordered newest first.
- The bell count reflects all unread or claimable events, capped visually at `9+`. Opening the panel does not mark anything read.
- Opening a row marks that event read and opens its existing web destination. A claimable reward stays visible until it is claimed or expires. “View all in Clash of Meat” opens the web history.
- No operating-system notifications or new in-client claim/duel actions. The web app keeps those decisions and their confirmations.

## Data and delivery

The app's `user_notifications` table is the source of truth. No new notification table or migration is needed. Add one bounded read model that returns the newest ten events where `read_at IS NULL OR claimable`, using the same `NotificationItem` mapping and `notificationHref` as the web page. Each item also carries a server-computed `sentence` from the web's `notificationSentence`, so reward wording cannot drift between clients. Return the existing attention summary alongside the items. The recipient is always derived from the installation bearer token, never from a client-supplied user ID.

Add the bounded snapshot to the authenticated heartbeat as an additive `notifications` field. The app already invalidates `companion:<user>` when notification state changes, and the desktop daemon already syncs on that topic; that gives the inbox a prompt refresh when connected, with its normal heartbeat as recovery. Keep the new field optional in the client so older app deployments remain usable during rollout.

Add a bearer-authenticated `POST /api/installations/notifications/read` accepting one numeric notification ID. Delegate to the existing `readNotifications(userId, { id })` operation. This is idempotent and owner-scoped. Return the refreshed bounded snapshot so a clicked row disappears promptly. The server continues to own expiry and claimability; the desktop does not infer them from timestamps.

The daemon forwards the bounded snapshot to the overlay on heartbeat and exposes a socket method to acknowledge one row. It does not persist message text in the hero artwork cache. The overlay renders the snapshot in memory, handles loading/offline/empty states, and opens only server-supplied same-app paths through its existing `open_app` command. Invalid or absent payloads do not erase the last in-memory good view during a temporary sync failure; disconnect and account switch clear it.

## Visual implementation

Reuse the desktop's walnut bubble, Righteous headings, Alegreya Sans body, beveled buttons, and existing pixel back arrow. Use the app's actual bell and notification icons as local client assets, following the web icon mapping. Place the bell beside the 128 px sprite using the same position calculation as the hero so it travels with the overlay; make it a separate hit region that does not start dragging the hero. The inbox uses the existing 360 × 380 px overlay window and scrolls within the space above the hero. Keep the whole panel within the screen work area and include its interactive rectangles in the overlay hit regions.

## Verification and rollout

- App: focused tests for the ten-item ordering/filter, read-but-claimable persistence, bearer ownership, and additive heartbeat contract.
- Client: focused Rust tests for heartbeat forwarding and socket acknowledgment; focused UI tests for bell count, panel states, row paths, and hit/size behavior. Inspect the actual Tauri overlay on macOS if available.
- Deploy the app change before the desktop release. Both changes are intended for their repositories' `dev` branches. Existing clients ignore the new heartbeat field; the new client tolerates an app that has not yet deployed it.
