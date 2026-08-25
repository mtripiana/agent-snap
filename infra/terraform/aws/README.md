# AgentSnap Disposable Demo VM (AWS)

This Terraform bundle provisions a disposable Ubuntu VM on AWS and runs the AgentSnap validation/demo automatically via cloud-init.

## Prerequisites

- Terraform >= 1.5
- AWS credentials configured for Terraform
- A reachable Git repository URL for this project

## Deploy

```bash
cd infra/terraform/aws
terraform init
terraform apply \
  -var="repo_url=https://github.com/<owner>/<repo>.git" \
  -var="my_ip_cidr=<YOUR_PUBLIC_IP>/32" \
  -var="ami_id=<PINNED_AMI_ID_FOR_YOUR_REGION>"
```

After apply, Terraform prints `public_ip`.

## Verify

```bash
ssh ubuntu@<PUBLIC_IP>
sudo tail -n 300 /var/log/cloud-init-output.log
sudo cat /root/agentsnap-demo-done.txt
```

Daemon log (optional):

```bash
sudo tail -n 200 /tmp/agentsnap-daemon.log
```

## Destroy

```bash
terraform destroy \
  -var="repo_url=https://github.com/<owner>/<repo>.git" \
  -var="my_ip_cidr=<YOUR_PUBLIC_IP>/32" \
  -var="ami_id=<PINNED_AMI_ID_FOR_YOUR_REGION>"
```

## Security Notes

- SSH ingress is restricted to `my_ip_cidr`.
- The AMI is an explicit required input so deployments cannot silently follow
  a moving image family.
- Do not place secrets or production credentials on this VM.
- Treat the machine as disposable; destroy it after testing.
