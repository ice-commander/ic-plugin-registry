# Registry Editor

An Ice Commander plugin that browses and edits the Windows registry inside a
file panel.

## What it does

It adds one button on the right of the panel toolbar (priority 30, icon only,
tooltip "Registry"). Pressing it opens the view `ic.registry` inside that panel
in place of the file list; the cross in the view's own toolbar puts the panel
back.

The view has a toolbar, two panes and a status line:

- toolbar: collapse all, refresh, new key, new value, edit value, delete, the
  path of the selected key, close;
- left: a tree of the five hives — HKLM, HKCU, HKCR, HKU, HKCC. A key's
  subkeys are read when its arrow opens it and dropped when it closes. Every
  key shows an arrow, so some open onto nothing. Selecting a key lists its
  values without opening it;
- right: the selected key's values in three columns — name, type, data —
  sorted by name. The unnamed value shows as `(Default)`; numbers as hex and
  decimal; binary as spaced hex; a multi-string as its entries joined by
  spaces. In the desktop application a value too long for its column is cut
  with an ellipsis and shown whole in the tooltip;
- status line: what was refused or failed, with the error Windows gave.

Changes:

- **New key** — under the selected key, asking for its name.
- **New value** — under the selected key, asking for a name and one of six
  types: `REG_SZ`, `REG_EXPAND_SZ`, `REG_MULTI_SZ`, `REG_DWORD`, `REG_QWORD`,
  `REG_BINARY`. It is created empty (`0` for numbers); select it and edit it
  to fill it in.
- **Edit value** — the button or activating a value. The type is kept. Numbers
  are typed in decimal or as `0x…` hex, a multi-string one entry per line
  (empty lines are dropped), binary as hex digits (whitespace ignored).
  Strings are saved as typed, surrounding spaces included.
- **Delete** — the selected value, or the selected key when no value is
  selected, after a confirmation.

Values in the root of a hive are read and written like any other. Every
question is drawn by the host through `ask`; the plugin has no dialogue code.

## What it registers

- eight SVG assets under `ic-registry-dlg`: `registry` and the seven toolbar
  icons;
- the view `ic.registry`, a JSON document with `surface: panel`, answering
  `describe` and `on_event`;
- the panel toolbar button, only when the host is the desktop application
  (`IC_HOST_GTK`, or a host that passes no kind).

No extensions, filesystems or locale catalogues.

## Known limitations

- The interface is English only. Labels carry `registry.*` translation keys,
  but the plugin registers no catalogue for them, so their English text is
  shown; status messages are plain English strings.
- Keys and values cannot be renamed.
- Deleting a key is not recursive: Windows refuses a key that has subkeys, and
  the refusal appears in the status line.
- Value types other than the six above (`REG_NONE`, `REG_LINK`,
  `REG_RESOURCE_LIST`, `REG_DWORD_BIG_ENDIAN` and so on), and a `REG_DWORD` or
  `REG_QWORD` shorter than its size, are listed as `REG_NONE` with no data and
  cannot be edited.
- The asset `registry` is registered but nothing refers to it by name; the
  toolbar button is given the same SVG directly.
- On Linux and macOS the library builds and loads and the button and the view
  appear, but the tree is empty and the status line says "the registry is a
  Windows thing".

## Building

```sh
./build.sh          # release build; the library is copied into bin/
./test.sh           # cargo test --workspace
./deploy-local.sh   # copies bin/ into the per-user plugin folder
```

`deploy-local.sh` installs into `~/Library/Application Support/ice-commander/plugins`
on macOS, `%APPDATA%\ice-commander\plugins` on Windows and
`${XDG_DATA_HOME:-~/.local/share}/ice-commander/plugins` elsewhere; set
`IC_PLUGIN_DIR` to use another folder. It removes only libraries it deployed
earlier that are no longer built. Then switch the plugin on in
**Settings → Plugins** and restart.

On Windows one test writes a value named `ic-registry-dlg selftest` into
`HKCU` and deletes it again.

The version comes from `package.json`; `node builder/gen-version.js` writes it
into `version.rs`. `ic-plugin-api` is taken from
`https://github.com/ice-commander/plugin-api.git` (branch `main`); `winreg` is
used on Windows only.

## Licence

MIT or Apache-2.0, at your option, except the SVG icons in
`src/registry-dlg/assets/`: they are from Icons8 and are not covered by these
licences — see [THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
Contributions are taken under the DCO; sign off with `git commit -s`.
