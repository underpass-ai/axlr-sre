#!/usr/bin/env bash
. "$(dirname "$0")/../lib.sh"
k -n "$ns" delete certificate shop-tls --ignore-not-found
k -n "$ns" delete issuer lab-selfsigned --ignore-not-found
k -n "$ns" delete secret shop-tls --ignore-not-found
echo "cert-expiry: cleared"
