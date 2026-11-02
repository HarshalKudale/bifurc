# Analytics

Bifurc sends anonymous usage analytics to PostHog. The module doc at the top of
`crates/engine/src/analytics/mod.rs` is the source of truth; this page is the short version.

## What is sent

What kind of thing was done, from which client and on which platform. **Never** a payload, a name, a URL or a
path.

- One event at most per command a client sends: what was done (`mock created`, `request sent`), the command's name,
  or `command failed` with the name and the error code.
- `engine failure` with a fixed word for what failed in the background.
- Crash reports: a panic's file and line in Bifurc's code, never the message.
- Every event carries the engine version, OS, architecture and `official_build`. An event for something done in a
  workspace also carries that workspace's random id, never its name.
- The only identifier is a random install id made for the data directory. There is no person profile, no location
  lookup and the IP address is not stored.

Nothing is sent before the terms of service and the privacy policy have been accepted.

## Builds you make yourself send nothing

The project key is compiled into the official releases from the release pipeline's secrets. A build without the
key sends no analytics at all. `official_build` is `true` only when the key was compiled in and no other key was
given when the engine ran.

## Opting out

- Set `DO_NOT_TRACK=1` or `BIFURC_TELEMETRY=0` in the environment, or
- turn it off in Settings (Anonymous usage analytics).

With the switch off, one event, `app installed`, is still sent once per data directory so that an install is
known of. Nothing the user does is sent.

See also [bifurc.app/privacy](https://bifurc.app/privacy).
