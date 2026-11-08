# Multi Claude

## Updates

The installed app updates itself from the GitHub Releases of this repository. It reads
`latest.json` of the newest published release 10 seconds after it starts and then once a day;
Settings → Window → **Check for updates automatically** turns that off, and **Check now** looks
right away. A development run (`npm run tauri dev`) and an isolated instance (`MULTI_CLAUDE_HOME`)
never check by themselves, so that they cannot install over the real app.

When a newer version is found, a bar above the tabs offers **What's new**, **Install and restart**
and **Later**. Installing downloads the update and checks its signature, ends the app the way
quitting does (the tabs are saved, the sessions end), installs it and starts the new version, which
reopens the tabs. On Windows the NSIS installer runs in passive mode, for the current user, without
an administrator prompt; on macOS the new app bundle takes the place of the old one.

### One-time setup

1. **Repository.** Releases come from the public repository `OWNER/multi-claude-rust`. Put the real
   owner in place of `OWNER` in `src-tauri/tauri.conf.json` → `plugins.updater.endpoints`. That URL
   is the only place the app takes it from. The repository must be public: the app downloads
   without credentials.
2. **Signing key.** Generate the key pair once:

   ```sh
   npx tauri signer generate -w ~/.tauri/multi-claude.key
   ```

   Choose a password when asked. This writes the private key `~/.tauri/multi-claude.key` and the
   public key `~/.tauri/multi-claude.key.pub`. Keep both the private key and its password safe and
   out of the repository. Without them no later update can be signed, and installed apps then have
   to be updated by hand with a new installer.
3. **Public key.** Paste the content of `multi-claude.key.pub` (one line) into
   `src-tauri/tauri.conf.json` → `plugins.updater.pubkey`, in place of
   `REPLACE_WITH_TAURI_SIGNER_PUBLIC_KEY`, and commit it. Apps built before this step cannot
   check any update.
4. **Secrets.** On GitHub, open the repository's Settings → Secrets and variables → Actions and add
   two repository secrets:
   - `TAURI_SIGNING_PRIVATE_KEY`: the whole content of `multi-claude.key`
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: the password from step 2

   `GITHUB_TOKEN` is provided by GitHub. The release workflow asks for `contents: write`; if the
   repository's Settings → Actions → General → Workflow permissions are set to read-only and the
   release cannot be created, allow read and write there.

### Making a release

1. Raise the version in `src-tauri/tauri.conf.json` (the app reports this one), and to keep them
   in step in `package.json` and the workspace `Cargo.toml`.
2. Add a section `## [x.y.z] - YYYY-MM-DD` to `CHANGELOG.md`. Its text becomes the release text
   and what the app shows under **What's new** before installing.
3. Commit, then tag and push the tag:

   ```sh
   git tag v0.6.1
   git push origin v0.6.1
   ```

   The tag must be `v` plus the version of `tauri.conf.json`; the workflow stops otherwise.
4. `.github/workflows/release.yml` builds Windows x64, macOS Apple silicon and macOS Intel one
   after the other and puts the installers, their `.sig` files and `latest.json` into a **draft**
   release.
5. Check the draft: `latest.json` lists `windows-x86_64`, `darwin-aarch64` and `darwin-x86_64`.
   Then publish it. Installed apps see the update from that moment; a draft or a prerelease is
   never offered.

`.github/workflows/ci.yml` runs the type check, the tests of the window and of the Rust workspace
and a build of the window on every push and pull request (Windows only).

### Building locally

`npm run dist` signs the updater files. It fails without the private key, and also while
`plugins.updater.pubkey` still holds the placeholder ("failed to decode pubkey"). Set the key for
the build:

```sh
TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/multi-claude.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="…" npm run dist
```

For a build without updater files, which cannot be published as an update:

```sh
npm run tauri build -- --config '{"bundle":{"createUpdaterArtifacts":false}}'
```
