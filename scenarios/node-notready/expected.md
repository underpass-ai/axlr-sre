# node-notready: expected facts

Alert: `LabNodeNotReady`, label `node=sre-lab-control-plane` (no namespace).

| Fact | Value |
|---|---|
| object | Node `sre-lab-control-plane` |
| condition | `Ready` is `Unknown` / `NotReady` |
| cause | the kubelet stopped: `Kubelet stopped posting node status` (every condition `Unknown`), not memory, disk or PID pressure |
| side effect | taint `node.kubernetes.io/unreachable`; its pods are marked not ready |
| fix (not applied) | restart the kubelet on the host (`systemctl start kubelet`); cordon the node meanwhile |
