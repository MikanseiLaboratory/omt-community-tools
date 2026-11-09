#!/usr/bin/env bash
set -euo pipefail

# Configure Tauri's macOS code signing for a GitHub Actions job.
#
# With a Developer ID Application certificate in the APPLE_CERTIFICATE secret,
# Tauri imports the certificate and (when Apple ID credentials are also set)
# notarizes the app. Without a certificate, fall back to ad-hoc signing.
#
# Ad-hoc signing is not optional on Apple Silicon. The linker already ad-hoc
# signs every arm64 Mach-O, so an unsigned .app looks like a bundle with a
# broken signature: Gatekeeper reports "code has no resources but signature
# indicates they must be present", shows "OMT Tools is damaged and can't be
# opened", and never starts the process.
#
# Secrets arrive as *_SECRET variables so this script also runs for pull
# requests from forks, where they are empty.

: "${GITHUB_ENV:?GITHUB_ENV is not set}"

write_env() {
  printf '%s=%s\n' "$1" "$2" >> "$GITHUB_ENV"
}

certificate="${APPLE_CERTIFICATE_SECRET:-}"
certificate_password="${APPLE_CERTIFICATE_PASSWORD_SECRET:-}"

if [[ -n "$certificate" || -n "$certificate_password" ]]; then
  if [[ -z "$certificate" || -z "$certificate_password" ]]; then
    echo "error: set both APPLE_CERTIFICATE and APPLE_CERTIFICATE_PASSWORD, or neither." >&2
    exit 1
  fi

  # Tauri decodes the certificate as standard base64, which does not accept
  # line breaks. `openssl base64` wraps its output by default, so strip any
  # whitespace before handing it to GITHUB_ENV.
  certificate="${certificate//[[:space:]]/}"
  write_env APPLE_CERTIFICATE "$certificate"
  write_env APPLE_CERTIFICATE_PASSWORD "$certificate_password"

  for name in APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID; do
    secret_name="${name}_SECRET"
    value="${!secret_name:-}"
    if [[ -n "$value" ]]; then
      write_env "$name" "$value"
    fi
  done

  if [[ -z "${APPLE_ID_SECRET:-}" || -z "${APPLE_PASSWORD_SECRET:-}" || -z "${APPLE_TEAM_ID_SECRET:-}" ]]; then
    echo "::warning title=macOS notarization::APPLE_CERTIFICATE is set but APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID are incomplete; the app will be signed but not notarized."
  fi

  echo "macOS signing: using the Developer ID certificate from repository secrets."
else
  write_env APPLE_SIGNING_IDENTITY "-"
  echo "::notice title=macOS signing::No APPLE_CERTIFICATE secrets; ad-hoc signing the app (users must allow it in Privacy & Security)."
fi
