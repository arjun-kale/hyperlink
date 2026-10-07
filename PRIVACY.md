# HyperLink Privacy Policy

_Last updated: 7 October 2026_

HyperLink links your Android phone to your own Linux computer. This policy
explains what data the apps access and where it goes. The short version:
**everything stays between your phone and your computer.** HyperLink has no
servers, no accounts, no analytics, no advertising, and no third-party SDKs.

## What HyperLink accesses, and why

HyperLink only uses each kind of data to provide the feature you turned on,
and sends it only to the computer you paired, over an encrypted connection on
your local network.

| Data | Feature | When |
|---|---|---|
| Notifications (app name, title, text, action buttons, icon) | Show your phone's notifications on your computer | Only if you grant Notification access |
| Screen contents | Show your phone's screen on your computer | Only while you're sharing the screen; Android asks each time |
| Clipboard contents | Copy on one device, paste on the other | While linked |
| Files on your phone's storage | Browse your phone's files from your computer | Only if you grant "All files access", and only files your computer asks for |
| Taps, swipes and key presses sent from your computer | Control your phone from your computer | Only if you enable HyperLink's Accessibility service |
| Do Not Disturb state | Keep Do Not Disturb in sync | Only if you grant Do Not Disturb access |
| Device name, screen on/off, charging state, media playing | Show which phone is linked; optional context features | While linked |
| Wi-Fi network information | Find your computer on the local network and keep the link working | While the app is looking for or connected to your computer |
| Android ID (a device identifier) | Tag clipboard items so a copy doesn't bounce back to the device it came from | Sent to your paired computer with clipboard items |

**Accessibility service.** HyperLink uses Android's Accessibility API for two
things, both only when you've enabled it:

1. To perform the taps, swipes, text entry and navigation (Back, Home, Recents)
   that you send from your own computer.
2. As a signal for clipboard sync: when a window changes or text is selected,
   HyperLink checks whether you copied something new, so copying on the phone
   reaches your computer even while HyperLink is in the background. Only the
   kind of event is used; HyperLink does not read the content of the event.

It cannot read your screen contents through Accessibility (it's configured
without window-content access) and does not use it for any other purpose.

## Where data goes

- **Only to your paired computer**, directly over your local network, encrypted
  with TLS 1.3. The two devices verify each other's certificates on every
  connection; pairing requires you to confirm a matching code on both.
- **Never to us or anyone else.** HyperLink does not send data to the
  internet. The only network request to the internet is the optional update
  check on the computer, which downloads the project's public changelog from
  GitHub to see whether a newer version exists. You can turn it off in
  Preferences.

## What's stored

- **On the phone:** its own pairing key and the list of computers it trusts,
  your onboarding progress, and a local activity log (Settings → Activity log).
  These are excluded from Android backups and device-to-device transfer.
- **On the computer** (in `~/.config/hyperlink` and `~/.local/share/hyperlink`):
  its pairing key, the phones it trusts, your preferences, window layout, notes
  sent from the phone, and, if you use the experimental context feature, a log
  of recent phone events.
- **Crash reports** (optional, on by default, can be turned off in
  Preferences) are saved only on the computer and never sent anywhere. They
  contain the error message and code location, never clipboard text,
  notification content, file names or keys.

Uninstalling HyperLink from the phone deletes its data. On the computer,
delete `~/.config/hyperlink` and `~/.local/share/hyperlink` to remove
everything.

## Children

HyperLink is not directed at children and does not knowingly collect any
personal data from anyone; data never leaves your own devices.

## Changes

If this policy changes, the new version will be published in this file with
an updated date.

## Contact

Questions or concerns: open an issue at
https://github.com/arjun-kale/hyperlink/issues, or see [SECURITY.md](SECURITY.md)
to report a security problem privately.
