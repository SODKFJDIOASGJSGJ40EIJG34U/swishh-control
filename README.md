# SWISHH CONTROL CENTER

A local-first Windows system dashboard built with Tauri, React, TypeScript, and Rust.

This repository contains the public source for SWISHH CONTROL CENTER. No personal device snapshot, machine-specific settings, monitoring history, or credentials are included. See the complete English and Russian documentation, screenshots, requirements, and build instructions in the full README in the source package. Screenshots are illustrative and contain no live device data.

The application reads selected system information locally from the Windows computer where it runs. It is independent and is not affiliated with Microsoft or hardware vendors.

## Build

Windows 10/11 x64, Node.js 22+, npm, Rust stable, and Tauri 2 Windows prerequisites are required.

```powershell
npm ci
npm run tauri -- build --bundles nsis
```

The NSIS installer is written to `src-tauri/target/release/bundle/nsis/`. GitHub Actions builds the installer for pushes and pull requests and publishes it on version tags (`v*`).

## Privacy

`.env` files, local logs, build outputs, and local state are excluded. Public screenshots use anonymized placeholder values. No license is included yet; public access does not grant reuse rights.
