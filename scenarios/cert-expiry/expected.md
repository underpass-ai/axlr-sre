# cert-expiry: expected facts

Alert: `LabCertificateExpiringSoon`, namespace `shop`, label `name=shop-tls`.

| Fact | Value |
|---|---|
| object | Certificate `shop-tls` (Secret `shop-tls`, DNS `shop.lab.internal`) |
| namespace | `shop` |
| expiry | `notAfter` about one hour after it was issued: expires in under 60 minutes |
| cause | `duration: 1h` on the Certificate (with `renewBefore: 10m`), issued by Issuer `lab-selfsigned` |
| fix (not applied) | set a real duration (for example `2160h`) and renew; or force a renewal now |
