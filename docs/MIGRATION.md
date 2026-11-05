# nabnab data migration

nabnab keeps the existing Tauri identifier, `com.ytnabber.desktop`. The visible product name and executable can change without losing the existing per-user configuration and managed yt-dlp updates. No new publisher, remote, update endpoint or signing identity is introduced by this migration.

The audited application stored only its selected download directory in `destination.json` under Tauri's `app_config_dir`. It did not persist a job history, collections or credentials. The new repository adds those local records in `state-v1.json` in the same directory. It does not fabricate legacy records, scan old download destinations, move media, rename files or delete old data. Engine update data remains in the existing `app_local_data_dir/engine-updates` directory.

## First start

1. Create the application data directory if needed and hold an exclusive `nabnab.lock` file handle until the repository closes. A second nabnab process cannot write the same repository.
2. If `state-v1.json` exists, parse and validate its schema and settings. Refuse malformed data and unknown versions without replacing them.
3. Otherwise read the real legacy `destination.json`, if present. Reject malformed or relative paths. Missing media directories remain references so the user can choose a new destination.
4. Write the legacy bytes to a newly created temporary file, flush it and atomically publish `destination.v0.backup.json` before committing migrated data. If a previous backup differs, stop and retain both files for review.
5. Construct schema version 1, recover interrupted jobs into a reviewable state, and serialize the complete repository to a newly created file in the same directory.
6. Flush, reopen and validate that temporary file. Commit it atomically to `state-v1.json`. On Windows the rename uses `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`.

The validated schema file is the completion marker. Later launches load it directly, so migration does not duplicate records or re-import a legacy destination. Failed writes retain the last committed state. A crash before publication can leave a `state-v1-<pid>-<nonce>.new` file; the app does not silently treat it as committed. Keep it for manual recovery if needed.

The old application does not know about the lock file. Its `destination.json` and the new repository are separate files, so it cannot write an incompatible schema. Changes made to the old application's destination after migration are not synchronized back to nabnab. Close the old version before upgrading. The NSIS upgrade and rollback still need installation testing on a separate Windows profile.

## Installer continuity

The default Tauri NSIS template derives its uninstall registry key and stored installation-directory key from `productName`. Keeping only `com.ytnabber.desktop` would therefore preserve application data but fail to detect the previous installation after renaming the product.

`src-tauri/installer.nsi` is based on the exact installed CLI version, `tauri-cli-v2.12.4`, upstream template blob `75efe3da953fb6914a08359705d94d9ee8732542`. It carries the full upstream MIT license notice. The changes preserve these technical keys:

- `Software\Microsoft\Windows\CurrentVersion\Uninstall\ytnabber`
- `Software\ytnabber\ytnabber`

All installer display strings, executable information, Add/Remove Programs display names and new shortcuts continue to use `nabnab`. The existing publisher derived from the original identifier is unchanged. The retained installation key restores an existing destination directory instead of creating a second installation. New installations use the current product name.

The installer checks both the current executable and an existing `ytnabber.exe` before replacing resources. It removes only the exact known legacy executable name. Desktop and Start Menu link migration checks that a legacy shortcut targets `$INSTDIR\ytnabber.exe` before modifying it. Links to unrelated executables are left alone. Existing matching links are updated in `/UPDATE` and `/NS` modes without creating shortcuts where none existed. Same-name shortcuts to unrelated applications are not overwritten; a conflicting legacy link keeps its filename but points to the new executable, leaving that rare name collision for the user to resolve.

Configure `bundle.windows.nsis.template` as `installer.nsi`. An installer-hooks override cannot safely replace these registry identities, because the upstream template includes hooks before defining the keys and compiles its reinstall detection before the install hook runs. On future Tauri CLI upgrades, compare the custom template to the matching upstream template and retain the registry and shortcut migration changes. Packaging compilation and a real current-user upgrade/uninstall remain separate required checks; reading or compiling this template does not certify them.

## Optional Windows preferences

Existing schema-1 settings gain `trayEnabled`, `closeToTray` and `startWithWindows` through serde defaults. All three default to false. Migration does not create a tray icon, change the close behavior or register an application to run at login. Choosing close-to-tray requires a tray icon to be enabled so the user can restore the window.

The single tray icon reuses the application emblem. Its menu restores and focuses the main window or explicitly exits nabnab; a left double-click restores the window. The main close handler may hide the window only when both saved tray preferences are enabled and the tray is available.

Windows login startup uses the current user's `Software\Microsoft\Windows\CurrentVersion\Run` value named `nabnab`. Its value is the fully quoted current executable followed by the fixed `--startup` argument. No shell, administrator privilege, scheduled task or unrelated login item is used. A conflicting value is preserved. Disabling this preference removes only an entry matching the application's own command.

Apply these preferences after explicit settings changes or from a previously saved choice. If applying a change or committing settings fails, restore the previous settings and call `preferences::apply` with the previous values. Debug verification using the isolated test-data directory skips real Windows startup registry writes. Unit tests exercise command construction and settings validation without enabling login startup.

## Library records

Imports reference selected local files. They do not copy, move, transcode, open or delete them. Folder import examines at most 10,000 entries per operation, skips unsupported file types, and rejects symlinks and Windows reparse points. It never scans the computer automatically. Canonical paths deduplicate repeated imports; IDs remain stable when a missing file is explicitly located again.

Collections and tags are organization metadata. Removing an item removes only its library record. A missing original remains listed until the user removes it or chooses its replacement. The original's organization and identity survive relocation.

FFprobe metadata is optional during import. Without a probe, codec and duration fields stay unknown. Verified download and tool outputs can supply actual probe results when indexing. Sources persist only their public HTTP/HTTPS origin, with credentials, path, signed query parameters and fragments removed. No cookies, tokens or credentials belong in this repository.

## Verification

Run `cargo test --manifest-path src-tauri/Cargo.toml` after integration. The `store` tests cover fresh installation, repeat launch, Unicode legacy paths, byte-preserving backups, malformed and future schemas, invalid settings, exclusive locking and lock release. The `library` tests cover deduplication, index-only removal, source sanitization, actual file sizes, inspector metadata, missing-file relocation, invalid labels and all-or-nothing import validation. A Unix-specific symlink test also guards traversal; Windows junction behavior needs a Windows fixture with the required filesystem privileges.
