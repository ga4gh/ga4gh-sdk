#!/usr/bin/env bash
# Answers a device login of the test Keycloak as its test user would in a
# browser, for cli/tests/keycloak_login.rs.
#
# Usage: approve-device.sh VERIFICATION_URI_COMPLETE [accept|deny]
#
# Opens the verification URL (which carries the user code), signs in with
# the test user of tests/keycloak/realm.json through the login form, then
# answers Keycloak's consent page with "Yes" (accept, the default) or "No"
# (deny). Keycloak has no API to approve a device code, so this script goes
# through the HTML forms of its default login theme; it fails if a page
# doesn't have the expected form.
#
# The credentials are those of the user in the throwaway test realm; they
# are for this test only.
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: $0 VERIFICATION_URI_COMPLETE [accept|deny]" >&2
  exit 2
fi
url=$1
answer=${2:-accept}
case $answer in
  accept) button=accept; label=Yes ;;
  deny) button=cancel; label=No ;;
  *) echo "Unknown answer: $answer" >&2; exit 2 ;;
esac
user=test-user
password=test-only-password

# scheme://host:port of the verification URL; form actions may be relative.
if ! [[ $url =~ ^(https?://[^/]+)/ ]]; then
  echo "Not an HTTP(S) URL: $url" >&2
  exit 2
fi
origin=${BASH_REMATCH[1]}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
jar=$work/cookies

# The action of the first form with `marker` in its tag, made absolute.
form_action() {
  local page=$1 marker=$2 action
  action=$(grep -o "<form[^>]*${marker}[^>]*>" "$page" | head -n 1 |
    grep -o 'action="[^"]*"' | sed -e 's/^action="//' -e 's/"$//' -e 's/&amp;/\&/g') || true
  if [[ -z $action ]]; then
    echo "No form with '$marker' on the page" >&2
    return 1
  fi
  if [[ $action == /* ]]; then
    action=$origin$action
  fi
  printf '%s\n' "$action"
}

fetch() {
  curl --silent --show-error --fail --location --max-time 30 \
    --cookie "$jar" --cookie-jar "$jar" "$@"
}

fetch --output "$work/login.html" "$url"
login_action=$(form_action "$work/login.html" 'id="kc-form-login"')
fetch --output "$work/consent.html" \
  --data-urlencode "username=$user" \
  --data-urlencode "password=$password" \
  --data-urlencode 'credentialId=' \
  "$login_action"

consent_action=$(form_action "$work/consent.html" 'login-actions/consent')
code=$(grep -o 'name="code" value="[^"]*"' "$work/consent.html" | head -n 1 |
  sed -e 's/.*value="//' -e 's/"$//') || true
if [[ -z $code ]]; then
  echo 'No consent code on the consent page' >&2
  exit 1
fi
fetch --output "$work/result.html" \
  --data-urlencode "code=$code" \
  --data-urlencode "$button=$label" \
  "$consent_action"

if [[ $answer == accept ]]; then
  expected='Device Login Successful'
else
  expected='Consent denied'
fi
if ! grep -q "$expected" "$work/result.html"; then
  echo "The last page does not say '$expected'" >&2
  sed -e 's/<[^>]*>//g' "$work/result.html" | grep -v '^[[:space:]]*$' | tail -n 5 >&2
  exit 1
fi
echo "Device login answered: $answer"
