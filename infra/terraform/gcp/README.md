# AgentSnap Disposable Demo VM (GCP)

This Terraform bundle provisions a disposable Ubuntu VM on GCP and runs the AgentSnap validation/demo automatically using a startup script.

## Prerequisites

- Terraform >= 1.5
- `gcloud` authenticated for your project
- Compute Engine API enabled
- A reachable Git repository URL for this project

## Deploy

```bash
cd infra/terraform/gcp
terraform init
terraform apply \
  -var="project_id=<GCP_PROJECT_ID>" \
  -var="repo_url=https://github.com/<owner>/<repo>.git" \
  -var="my_ip_cidr=<YOUR_PUBLIC_IP>/32" \
  -var="image=<PINNED_UBUNTU_24_04_IMAGE>"
```

After apply, Terraform prints `public_ip`.

## Verify

```bash
gcloud compute ssh agentsnap-disposable-demo --zone <YOUR_ZONE>
sudo tail -n 300 /var/log/syslog
sudo cat /root/agentsnap-demo-done.txt
```

Daemon log (optional):

```bash
sudo tail -n 200 /tmp/agentsnap-daemon.log
```

## Destroy

```bash
terraform destroy \
  -var="project_id=<GCP_PROJECT_ID>" \
  -var="repo_url=https://github.com/<owner>/<repo>.git" \
  -var="my_ip_cidr=<YOUR_PUBLIC_IP>/32" \
  -var="image=<PINNED_UBUNTU_24_04_IMAGE>"
```

## Security Notes

- SSH ingress is restricted to `my_ip_cidr`.
- The image is an explicit required input so deployments cannot silently
  follow a moving image family.
- Do not place secrets or production credentials on this VM.
- Treat the machine as disposable; destroy it after testing.
