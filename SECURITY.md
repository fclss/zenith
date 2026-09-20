# Security Policy

## Reporting a vulnerability

Please **do not** open a public issue for security problems. Report them
privately via GitHub's [security advisories](https://github.com/fclss/zenith/security/advisories/new)
or by email to the maintainer listed on the GitHub profile.

Include as much as you can:

- affected version / commit,
- a description of the issue and its impact,
- steps to reproduce, and
- any suggested fix.

You can expect an acknowledgement within a few days and a coordinated fix and
disclosure once the issue is understood.

## Scope

zenith is a control surface that spawns `sing-box` and, in TUN mode, runs that
core elevated. Areas of particular interest:

- how the client handles subscription contents (it must never execute them),
- privilege boundaries (`sudo` invocation, generated config, file permissions),
- anything that could route traffic outside the tunnel unintentionally.

## Supported versions

Only the latest release is supported with security fixes.
