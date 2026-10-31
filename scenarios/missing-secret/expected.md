# missing-secret: expected facts

Alert: `LabContainerConfigError`, namespace `shop`.

| Fact | Value |
|---|---|
| object | Deployment `orders` (container `orders`) |
| namespace | `shop` |
| state | `CreateContainerConfigError` |
| cause | Secret `db` not found in `shop` (`secret "db" not found`) |
| reference | env `DB_PASSWORD` from `secretKeyRef` `db` / `password` |
| fix (not applied) | recreate the Secret `db` with the key `password` (a person does it; the bot never creates secrets) |
