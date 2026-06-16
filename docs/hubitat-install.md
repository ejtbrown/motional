# Hubitat Installation

1. In Hubitat, open **Apps Code** and create a new app from `hubitat/Motional.groovy`.
2. Save the app. If Hubitat exposes an OAuth toggle for the app code, enable OAuth so mapped app endpoints can be used.
3. Open **Apps**, add **Motional**, and select the motion and presence sensors that should be available.
4. Create a Motional token and select the sensor API names that token may access.
5. Copy the Hubitat endpoint URL and the Motional token shown by the app.

Requests use two tokens when Hubitat endpoint OAuth is enabled:

- Hubitat app endpoint token: passed as `access_token=<...>` in the URL.
- Motional token: passed as `Authorization: Bearer <...>`.

```sh
curl \
  -H "Authorization: Bearer <motional-token>" \
  "http://hubitat.local/apps/api/123/office?access_token=<hubitat-app-access-token>"
```

The `<sensor-name>` path segment is the API name shown in the Motional app. It is derived from the device display name by lowercasing it and replacing non-alphanumeric characters with dashes. If two selected devices produce the same API name, Motional appends the Hubitat device id to keep names unique.

