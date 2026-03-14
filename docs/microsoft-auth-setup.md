# Microsoft Authentication Setup for yammm

## Why a dedicated Azure AD app is needed

yammm uses the Microsoft OAuth2 device code flow to authenticate Minecraft players through their Microsoft account. This requires registering an application in Microsoft Entra ID (formerly Azure AD).

## Creating the Azure AD application

1. Go to [Microsoft Entra admin center](https://entra.microsoft.com) → **App registrations** → **New registration**
2. **Name**: `yammm`
3. **Supported account types**: "Accounts in any organizational directory and personal Microsoft accounts"
4. **Redirect URI**: Select **Mobile and desktop applications** → set to `https://login.microsoftonline.com/common/oauth2/nativeclient`
5. Click **Register**
6. Note the **Application (client) ID** — this becomes `MS_CLIENT_ID` in the code

### Required configuration

- **Authentication** → Add platform → **Mobile and desktop applications** → enable the native client redirect URI
- **No client secret needed** — this is a public client using device code flow

### API permissions

The `XboxLive.signin` scope is a **first-party Microsoft permission** that is not available through the standard API permissions page in the Azure portal. It is implicitly requested via the `scope` parameter at auth time.

## Applying for Minecraft API access

**This step is critical.** Without it, `api.minecraftservices.com` will return HTTP 403.

New Azure AD applications must be reviewed and approved by Microsoft before they can authenticate Minecraft accounts. Submit your app for review using the official form:

**https://aka.ms/mce-reviewappid**

The form asks for:
- Your Azure AD **Application (client) ID**
- The app's **public source repository URL** (the app must be open-source and publicly accessible)
- A description of what the app does

Until Microsoft approves the application, authentication will fail at the Minecraft services step with a 403 error.

## Important notes

- You **must** use the `consumers` AAD tenant (`login.microsoftonline.com/consumers/`) — using an Azure AD tenant ID or `common` will produce errors with the `XboxLive.signin` scope
- Only consumer Microsoft accounts (personal) can sign in — organizational accounts are not supported
- The app must be published as open-source before applying for review
- The `XboxLive.signin` scope does not appear in the Azure portal's API permissions — it is requested dynamically at auth time

## Current status

- **Client ID**: `31c26fc2-ce20-4fa9-95ca-21ecb8fd231b`
- **Approval status**: Pending — need to publish the repo and submit the review form

## References

- [Microsoft authentication — Minecraft Wiki](https://minecraft.wiki/w/Microsoft_authentication)
- [App review form](https://aka.ms/mce-reviewappid)
- [OAuth2 device code flow — Microsoft Docs](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-device-code)
