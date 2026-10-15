---
title: the engine server
order: 30
summary: Run Ethereal's audio engine on another machine and drive it from this window.
platform: all
audience: advanced
covers:
  - remote-engine/engine-server-remote-engine #connecting-to-an-engine-server
  - remote-engine/what-runs-where-with-an-engine-server #what-runs-where
  - remote-engine/engine-server-security #security
  - remote-engine/uploading-audio-to-an-engine-server #adding-audio-files
  - remote-engine/remote-engine-latency #latency-when-the-engine-is-elsewhere
  - remote-engine/audio-device-released-with-no-engine #when-your-own-engine-is-off
---

Normally the audio engine (the part that plays and processes sound) runs inside the app or the browser tab. An engine server, `ether-server`, is that same engine running on its own, without a window, on another computer: a studio machine with your plugins and audio interface, for example. This window then becomes a remote control for it. This is an advanced setup: most people never need it.

## connecting to an engine server

`ether-server` is a separate program. It isn't part of the download yet, so you build it from Ethereal's source code. When it starts, it prints its address and where to find its token.

Because the server isn't part of the download, its settings are hidden until you ask for them:

1. Open the [command palette](/docs/basics/command-palette-and-settings) and run **engine server…**. In the web app, you can instead add `?engine-server` to the address. This opens **settings** on the **advanced** tab, where the **engine server** section now stays visible.
2. Under **engine server**, enter the **address** (`ws://host:port`) and the **token**.
3. Click **connect**.

While connected, the top bar shows **●** and the server's name. Click it to see the server's version, or to **disconnect**. Disconnecting goes back to this window's own engine. If the connection drops, the window goes back to its own engine by itself and says that the connection was lost.

The window remembers the address, but never the token.

## what runs where

With an engine server, the project, the plugins and all the audio processing live on the server. Sound comes out of the **server's** audio device, not your computer's speakers. Your window only shows the project and sends your edits.

Several windows can connect to the same server at once. They share one project and see each other's edits live.

## adding audio files

Your audio files have to reach the server. While connected, drop audio files onto the browser: they are uploaded to the server and imported into the project. The top bar shows the progress of each upload. If you disconnect before an upload finishes, that upload is lost.

## security

- The server only listens on its own machine (`127.0.0.1`) by default. To accept other machines, it must be started with `--listen`, and it then requires a token.
- Over anything but a network you trust, put the server behind TLS (a reverse proxy).

When a connection is refused, the settings say why:

| message | what it means |
|---|---|
| wrong or missing token | check the token the server printed. |
| this server runs an incompatible version of ethereal | the server and this window must be the same version. |
| the server does not accept more clients right now | the server is busy: try again later. |

## latency when the engine is elsewhere

When you hear an engine on another computer (an engine server, or a session's host playing for you: see [listening on a friend's computer](/docs/sharing/listening-together)), every play or stop makes a round trip over the network, plus the time to encode and buffer the audio. That's tens of milliseconds on a local network and more over the internet. It is fine for playback and mixing, not for playing an instrument live.

Live MIDI, the [computer midi keyboard](/docs/making-music/computer-midi-keyboard) and recording still go to your own engine, not to the remote one.

## when your own engine is off [[desktop]]

When your own engine is turned off (you chose no engine, or you play through someone else's), the desktop app closes your audio device so other apps can use it. The transport bar's engine status then reads **closed**, and the audio settings say so too. The device opens again as soon as your engine is back on.

## where next

- [ai agents (mcp)](/docs/advanced/mcp): `ether-mcp` can drive an engine server too.
- [listening on a friend's computer](/docs/sharing/listening-together)
- [audio and midi setup](/docs/making-music/audio-and-midi-setup)
