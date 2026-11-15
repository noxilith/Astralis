# OneDrive setup

Astralis connects to OneDrive with **your own Microsoft app registration**, so no one else's server is involved. It's free, takes about five minutes, and you only copy one ID (there is no secret).

Cloud vaults on OneDrive work like the Google Drive ones: a vault is `Astralis/<vault name>/` in your OneDrive, holding only end-to-end encrypted files. You can add the vault on your other devices from the drive alone, and invite people by email.

## 1. Register the app

You need a Microsoft Entra directory to register an app. A free Azure account creates one (Microsoft asks for a card to verify you; registering costs nothing). A work or school account that may register apps works too.

1. Open <https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade> and choose **New registration**.
2. **Name:** Astralis.
3. **Supported account types:** "Accounts in any organizational directory and personal Microsoft accounts". Without this, personal OneDrive accounts can't sign in.
4. **Redirect URI:** platform **Public client/native (mobile & desktop)**, value `http://localhost`. Click **Register**.
5. **API permissions → Add a permission → Microsoft Graph → Delegated permissions:** `Files.ReadWrite`, `offline_access`, `User.Read`.
6. **Authentication → Advanced settings:** set **Allow public client flows** to **Yes**.
7. Copy the **Application (client) ID** from the Overview page.

## 2. Give Astralis the ID

- **Your own copy:** Settings → Connected drives → Microsoft OneDrive → **Set up**, paste the ID, **Save**, then **Connect**.
- **Release builds:** set the `ASTRALIS_ONEDRIVE_CLIENT_ID` repository secret (or the environment variable when building). Then everyone who installs that build can just click **Connect**.

Everyone you invite uses Astralis with the same app registration, so share the ID with them if they run their own build. A client ID isn't secret, and it can't reach anyone's OneDrive by itself.

## Using it

- **New cloud vault** (vault menu): pick OneDrive.
- **Add existing cloud vault:** lists the vaults in your OneDrive; the device that made one must have been opened once with a version that supports OneDrive.
- **Invite:** vault menu → People. They get an email from OneDrive, accept it, then choose **Join shared vault from OneDrive…** in Astralis.

## How it differs from Google Drive

- OneDrive folders have no private properties, so a vault's details (which vault, which device owns it, invites) live in a small `.astralis-props.json` file inside its folder. The vault keys for your own devices are in the same kind of file in the top-level `Astralis` folder, which is never shared.
- Astralis asks for the `Files.ReadWrite` permission, because a vault shared with you lives in the other person's OneDrive and the app-folder-only permission can't reach it. Astralis only reads and writes inside `Astralis/` folders and the vaults shared with you.
