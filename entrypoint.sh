#!/bin/sh
# safe entrypoint: read secrets into env vars (avoid trailing newlines)
if [ -f /run/secrets/google_service_account_json ]; then
  export GOOGLE_SERVICE_ACCOUNT_JSON=$(cat /run/secrets/google_service_account_json)
fi
if [ -f /run/secrets/backend_admin_secret_key ]; then
  export BACKEND_ADMIN_SECRET_KEY=$(cat /run/secrets/backend_admin_secret_key)
fi

# run original command (replace with your app path if different)
exec "$@"
