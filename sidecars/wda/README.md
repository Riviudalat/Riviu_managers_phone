# Riviumanagersphone on-device agent

Branded WebDriverAgent installed on each iPhone.

| Field | Value |
|-------|--------|
| Display name | Riviumanagersphone |
| Bundle ID | com.riviu.managersphone.agent |
| Icon | Orange R from repo `logo.jpg` |

## Requirement

Full **Xcode.app** (App Store) + Apple ID in Xcode Accounts with an **Apple Development** certificate.
Command Line Tools alone is not enough.

```bash
# After Xcode is installed and Apple ID is added:
sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
python3 sidecars/wda/build_and_install.py --udid <UDID>
```

Or use the desktop button **Cài Riviumanagersphone**.

Free profiles expire ~7 days — re-run install to refresh.
