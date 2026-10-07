# Setting up HyperLink

This guide takes you from nothing to a phone linked with your computer, and
covers what to do when something doesn't work. For a shorter version, see the
[README](../README.md).

## 1. Install on your computer

Download `hyperlink-linux-<version>-x86_64.tar.gz` from the
[latest release](https://github.com/arjun-kale/hyperlink/releases/latest).

HyperLink uses your system's GTK and GStreamer libraries. On Ubuntu or Debian:

```sh
sudo apt install gstreamer1.0-plugins-bad gstreamer1.0-libav gstreamer1.0-gtk4 \
  wl-clipboard fuse3 libnotify-bin
```

For hardware video decoding (smoother screen sharing, much less CPU), also
make sure GStreamer's VA plugin is installed. It's part of
`gstreamer1.0-plugins-bad` on Ubuntu 24.04 and of `gstreamer1.0-plugins-extra`
on Ubuntu 26.04.

Then install the app for your user (no `sudo`):

```sh
tar xzf hyperlink-linux-*-x86_64.tar.gz
cd hyperlink-linux-*-x86_64
./linux/packaging/install-local.sh --no-build
```

Open **HyperLink** from your apps. To remove it later:
`./linux/packaging/install-local.sh --uninstall` (your pairings are kept in
`~/.config/hyperlink`).

**Start when you log in:** HyperLink → menu → Preferences → *Start When You
Log In*. Closing the window keeps HyperLink running in the background so your
phone stays linked; open it again from your apps to bring the window back.

## 2. Install on your phone

Download `HyperLink-<version>.apk` on the phone and open it.

### Installing the APK

Android is stricter about apps installed from a file than from an app store,
and HyperLink asks for two sensitive permissions (notification access and
accessibility), so you may see one of these:

| What you see | Why | What to do |
|---|---|---|
| "Install unknown apps" prompt | The app you opened the APK from (browser, Files, WhatsApp) isn't allowed to install apps | Allow it for that app, then tap the APK again |
| "Blocked by Auto Blocker" (Samsung) | Samsung blocks installs from outside the Galaxy and Play stores | Settings → Security and privacy → **Auto Blocker** → off, install, then turn it back on if you like |
| "App blocked to protect your device" (Play Protect) | Google blocks file-installed apps that request notification or accessibility access (enhanced fraud protection, on in some countries including India) | Install over USB instead (below), or temporarily turn off Play Store → profile → Play Protect → ⚙ → *Scan apps with Play Protect*, install, and turn it back on |
| "App not installed" | A HyperLink signed with a different key is already installed | Uninstall the old HyperLink first |

**Over USB** (always works): enable Developer options → USB debugging on the
phone, connect it, and on the computer run
`adb install HyperLink-<version>.apk`.

## 3. Pair them (once)

1. Make sure the phone and the computer are on the **same Wi-Fi network**.
2. On the computer, open HyperLink and click **Start Pairing**. Pairing stays
   open for 5 minutes.
3. On the phone, open HyperLink, tap **Get started**, and tap your computer in
   the list. It appears under your computer's name.
4. The phone shows a 6-digit code. The computer shows three codes: **click the
   one that matches the phone**, then tap **Pair** on the phone.

Picking the code is what proves you're pairing your own two devices, not a
stranger's on the same network. Pairing is saved on both devices only when
both of you confirm.

## 4. Turn on what you want to use

The phone's home screen lists each feature and opens the right Android setting
when you tap **Allow** or **Turn on**:

- **Notifications** — your phone's notifications appear on the computer.
- **Control from PC** — Accessibility permission, so the computer's mouse and
  keyboard can operate the phone.
- **Files** — "All files access", so the computer can browse your phone's
  storage (it appears in your file manager while linked).
- **Do Not Disturb sync** — muting one device mutes both.
- **Clipboard** — always on.

**To see the phone's screen on the computer**, tap **Show screen on PC** on the
phone and accept Android's screen-sharing prompt. On the computer, click to
tap, scroll to swipe, type to enter text; Esc or right-click is Back, the
Super key is Home. **Stop showing screen** on the phone ends it.

## Daily use

- They reconnect by themselves when both are on the same Wi-Fi. The phone
  shows a **Linked to …** notification while connected (tap it to open the
  app, or use its **Disconnect** button).
- If the connection drops (Wi-Fi blip, computer asleep), the phone keeps trying
  for 5 minutes in the background, even with the app closed.
- **Removing a phone:** on the computer, Preferences → Paired Phones → Remove.
  It's disconnected immediately and has to pair again to reconnect.

## Troubleshooting

**The phone doesn't find the computer**
- Both must be on the same Wi-Fi network. Guest and public Wi-Fi often block
  devices from seeing each other.
- Make sure HyperLink is running on the computer (open it from your apps).
- If your computer has a firewall enabled, allow HyperLink's ports:
  `sudo ufw allow 9900/udp && sudo ufw allow 9901/udp && sudo ufw allow 5353/udp`.

**"… isn't ready to pair" on the phone** — click **Start Pairing** on the
computer first, then tap the computer on the phone.

**"… doesn't recognise this phone anymore"** — the phone was removed on the
computer (or the computer's HyperLink was reinstalled). Pair again.

**"Different versions of HyperLink"** — update both apps to the same release.

**Screen sharing is slow or blurry**
- Install the hardware decoding package (step 1) and restart HyperLink.
- A 5 GHz Wi-Fi network helps a lot; 2.4 GHz is often congested.

**Clipboard from the computer doesn't reach the phone** — on GNOME, copy while
the HyperLink window is focused (GNOME only tells the focused app about
clipboard changes).

**Files don't show up** — allow *Files* on the phone, and make sure `fuse3` is
installed on the computer. The phone's storage is mounted at
`/run/user/<your id>/hyperlink` while linked; the **Open** button on the
computer's home screen opens it.

## Advanced

Command-line options for the Linux app:

| Option | Default | What it does |
|---|---|---|
| `--name <name>` | your computer's name | The name phones see |
| `--bind <addr>` | `0.0.0.0:9900` | Address to listen on |
| `--background` | off | Start without showing the window (used by "Start when you log in") |
| `--config <path>` | `~/.config/hyperlink/host_config.json` | Settings and paired phones |
| `--pair` | off | Open pairing at startup (headless use) |

Logs: run `hyperlink-linux` from a terminal, or with `RUST_LOG=debug` for more
detail. On the phone, Settings (gear icon) → Activity log.
