# image-pull: expected facts

Alert: `LabImagePullBackOff`, namespace `shop`.

| Fact | Value |
|---|---|
| object | Deployment `catalog` (container `catalog`) |
| namespace | `shop` |
| state | `ImagePullBackOff` / `ErrImagePull` |
| image | `docker.io/library/nginx:1.99` |
| cause | the tag `1.99` does not exist in the registry (`not found`), not a credentials problem |
| fix (not applied) | set an existing tag (for example `nginx:1.27-alpine`) |
