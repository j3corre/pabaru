# Releasing Pabaru

## Making a release
1. Update `CHANGELOG.md` (new `## [X.Y.Z] – date` section; its text becomes the release notes).
2. Bump `version` in the root `Cargo.toml`, run `cargo check` (updates `Cargo.lock`), commit.
3. Tag and push: `git tag -a vX.Y.Z -m "Pabaru X.Y.Z" && git push origin main vX.Y.Z`.
4. `.github/workflows/release.yml` builds all packages and creates a **draft** release.
   Check it, then click *Publish release*. The website links to
   `releases/latest/download/<name>`, so it serves the new version immediately.

A manual run of the *Release* workflow (*Actions → Release → Run workflow*) builds the
packages as workflow artifacts only, without creating a release.

## macOS signing and notarization
Current macOS versions refuse to open downloaded apps that are not signed with a
**Developer ID** certificate and notarized by Apple; ad-hoc signed apps can't be opened
from the Finder at all. The release workflow signs and notarizes the app, the `.dmg` and
the macOS CLI when these repository secrets are set; without them it falls back to an
ad-hoc signature (fine for local testing and forks, not for distribution).

| Secret | Value |
|---|---|
| `APPLE_CERTIFICATE` | the Developer ID Application certificate + private key as `.p12`, base64-encoded |
| `APPLE_CERTIFICATE_PASSWORD` | password of the `.p12` file |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Jan Bláha (AB12CD34EF)` |
| `APPLE_ID` | Apple Account e-mail used for the developer membership |
| `APPLE_PASSWORD` | an **app-specific password** for that Apple Account (not the account password) |
| `APPLE_TEAM_ID` | the 10-character Team ID |

### One-time setup
1. **Apple Developer Program:** enroll at <https://developer.apple.com/programs/enroll/>
   (individual membership, USD 99/year). Approval can take up to two days.
2. **Certificate:** on a Mac, open *Xcode → Settings → Accounts*, select the team,
   *Manage Certificates… → + → Developer ID Application*. (Without Xcode: create a CSR in
   *Keychain Access → Certificate Assistant → Request a Certificate From a Certificate
   Authority*, upload it at <https://developer.apple.com/account/resources/certificates/add>
   as *Developer ID Application* (G2 Sub-CA), download and double-click the certificate.)
3. **Export:** in *Keychain Access → login → My Certificates*, right-click
   *Developer ID Application: …* → *Export…* → `pabaru-developer-id.p12` with a strong password.
   The entry must include the private key (expand the triangle to check).
4. **Identity and Team ID:**
   ```sh
   security find-identity -v -p codesigning
   # 1) 0123…  "Developer ID Application: Jan Bláha (AB12CD34EF)"
   #    identity = the quoted text, Team ID = the text in parentheses
   ```
5. **App-specific password:** <https://account.apple.com> → *Sign-In and Security →
   App-Specific Passwords* → create one named "pabaru-notarization".
6. **Store the secrets** (GitHub CLI, from the folder with the `.p12`):
   ```sh
   R=j3corre/pabaru
   base64 -i pabaru-developer-id.p12 | gh secret set APPLE_CERTIFICATE -R $R
   gh secret set APPLE_CERTIFICATE_PASSWORD -R $R      # paste the .p12 password
   gh secret set APPLE_SIGNING_IDENTITY -R $R          # paste the identity
   gh secret set APPLE_ID -R $R
   gh secret set APPLE_PASSWORD -R $R                  # paste the app-specific password
   gh secret set APPLE_TEAM_ID -R $R
   ```
   Then delete the exported `.p12` (the certificate stays in your keychain).

The workflow fails if Gatekeeper does not accept the signed and notarized app
(`spctl -a -t exec`), so a broken signature never reaches a release.

### Check a downloaded build
```sh
spctl -a -vv -t exec /Applications/Pabaru.app      # → accepted, source=Notarized Developer ID
xcrun stapler validate /Applications/Pabaru.app
```

## Windows
The installers are not signed yet, so SmartScreen shows a warning (*More info → Run
anyway*). Signing needs a code-signing certificate (e.g. Azure Trusted Signing or an OV/EV
certificate) and `sign-command` in `[package.metadata.packager.windows]`.
