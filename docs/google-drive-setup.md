# Connecting Google Drive

Astralis signs in to Google with **your own** OAuth client, so no server of ours is involved.
Setting one up is free and takes about five minutes. You do it once; everyone you invite then
uses the same client ID and secret.

## 1. Create a Google Cloud project

1. Open <https://console.cloud.google.com/projectcreate>.
2. Name it (for example "Astralis") and click **Create**. Make sure it's selected in the project
   picker at the top.

## 2. Turn on the two APIs

- Google Drive API: <https://console.cloud.google.com/apis/library/drive.googleapis.com>, then **Enable**.
- Google Picker API: <https://console.cloud.google.com/apis/library/picker.googleapis.com>, then **Enable**.
  Friends use the Picker to choose the vault folder you shared.

## 3. Set up the consent screen

Open <https://console.cloud.google.com/auth/branding> and click **Get started**:

1. **App information:** app name "Astralis" and your email as the support email.
2. **Audience:** **External**.
3. **Contact information:** your email. Agree to the policy, then **Create**.

Then:

- **Data access**, **Add or remove scopes**: add
  `https://www.googleapis.com/auth/drive.file` ("See, edit, create and delete only the specific
  Google Drive files you use with this app"). Click **Update**, then **Save**.
- **Audience**, **Publish app**. `drive.file` is a non-sensitive scope, so this doesn't need
  Google's security review. If you leave the app in *Testing*, Google expires the sign-in every
  7 days, and only accounts listed as test users can sign in. That includes friends you invite.

## 4. Create the desktop client

1. Open <https://console.cloud.google.com/auth/clients/create>.
2. Set **Application type** to **Desktop app**, give it any name, and click **Create**.
3. Copy the **Client ID** (ends in `.apps.googleusercontent.com`) and the **Client secret**
   (starts with `GOCSPX-`).

Google treats a desktop app's secret as non-confidential. On its own it can't open anyone's
Drive; each person still signs in and approves access.

## 5. Connect in Astralis

**Settings → Connected drives → Google Drive → Set up**. Paste the client ID and secret, click
**Save**, then **Connect**. Your browser opens; sign in and allow access.

## Using it

- **New cloud vault:** in the vault menu, choose **New cloud vault… → Google Drive**. This creates
  `Astralis/<name>/` in your Drive.
- **Invite someone:** in the vault menu, choose **Invite by email…**. Google emails them the
  folder.
- **Joining (the person you invited):**
  1. Set up Astralis with the same client ID and secret (step 5).
  2. In the vault menu, choose **Join shared vault from Google Drive…**.
  3. Pick the folder in the browser.

  Your Astralis has to be open the first time they join, because the invite is checked
  device-to-device. After that you edit together live, with no pairing code.
- **Removing someone:** the trash button next to them in **Invite by email…** removes their Drive
  access and their copy of the vault.

## What's stored where

- **On the Drive:** only end-to-end encrypted sync files (`*.astr`). Google sees ciphertext, plus
  the vault's name as the folder name.
- **The vault's key:** never on the Drive. It goes from your device to members over the
  encrypted P2P connection.
- **Your own devices** signed in to the same Google account sync through the Drive even when
  they're never online together.
- **Between different people,** edits flow P2P whenever two members are online. Google only lets
  each person's copy of Astralis read Drive files it made itself or that the person picked (that
  is how `drive.file` works). So the Drive acts as the offline mailbox for your own devices.
- **Only the folder's owner uploads.** Files a member put in your folder would be unreadable to
  everyone else, and Google refuses to change who a folder is shared with while it holds files your
  copy of Astralis can't touch. So members don't upload (and Astralis 1.0.2+ removes any files an
  older version uploaded).
- **The sign-in token** is kept in this device's Astralis database. **Disconnect** removes it and
  revokes it at Google.
