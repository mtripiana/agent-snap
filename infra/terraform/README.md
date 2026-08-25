# AgentSnap Disposable Cloud Demos

This directory contains Terraform bundles for running AgentSnap in disposable cloud VMs.

- AWS bundle: `infra/terraform/aws`
- GCP bundle: `infra/terraform/gcp`

These bundles are designed for safe validation away from your local workstation.

## Makefile Shortcuts (from repo root)

```bash
make help
make demo-aws-init
make demo-aws-apply REPO_URL=https://github.com/<owner>/<repo>.git MY_IP_CIDR=<YOUR_PUBLIC_IP>/32 AMI_ID=<PINNED_AMI_ID>
make demo-aws-destroy REPO_URL=https://github.com/<owner>/<repo>.git MY_IP_CIDR=<YOUR_PUBLIC_IP>/32 AMI_ID=<PINNED_AMI_ID>

make demo-gcp-init
make demo-gcp-apply PROJECT_ID=<GCP_PROJECT_ID> REPO_URL=https://github.com/<owner>/<repo>.git MY_IP_CIDR=<YOUR_PUBLIC_IP>/32 IMAGE=<PINNED_IMAGE>
make demo-gcp-destroy PROJECT_ID=<GCP_PROJECT_ID> REPO_URL=https://github.com/<owner>/<repo>.git MY_IP_CIDR=<YOUR_PUBLIC_IP>/32 IMAGE=<PINNED_IMAGE>
```

## AWS Quickstart

```bash
cd infra/terraform/aws
terraform init
terraform apply \
  -var="repo_url=https://github.com/<owner>/<repo>.git" \
  -var="my_ip_cidr=<YOUR_PUBLIC_IP>/32" \
  -var="ami_id=<PINNED_AMI_ID_FOR_YOUR_REGION>"
```

Get VM IP from Terraform output:

```bash
terraform output public_ip
```

Verify run completion:

```bash
ssh ubuntu@<PUBLIC_IP>
sudo tail -n 300 /var/log/cloud-init-output.log
sudo cat /root/agentsnap-demo-done.txt
```

Destroy resources:

```bash
terraform destroy \
  -var="repo_url=https://github.com/<owner>/<repo>.git" \
  -var="my_ip_cidr=<YOUR_PUBLIC_IP>/32" \
  -var="ami_id=<PINNED_AMI_ID_FOR_YOUR_REGION>"
```

## GCP Quickstart

```bash
cd infra/terraform/gcp
terraform init
terraform apply \
  -var="project_id=<GCP_PROJECT_ID>" \
  -var="repo_url=https://github.com/<owner>/<repo>.git" \
  -var="my_ip_cidr=<YOUR_PUBLIC_IP>/32" \
  -var="image=<PINNED_UBUNTU_24_04_IMAGE>"
```

Get VM IP from Terraform output:

```bash
terraform output public_ip
```

Verify run completion:

```bash
gcloud compute ssh agentsnap-disposable-demo --zone <YOUR_ZONE>
sudo tail -n 300 /var/log/syslog
sudo cat /root/agentsnap-demo-done.txt
```

Destroy resources:

```bash
terraform destroy \
  -var="project_id=<GCP_PROJECT_ID>" \
  -var="repo_url=https://github.com/<owner>/<repo>.git" \
  -var="my_ip_cidr=<YOUR_PUBLIC_IP>/32" \
  -var="image=<PINNED_UBUNTU_24_04_IMAGE>"
```

## Notes

- Keep these VMs disposable and free of production credentials.
- Restrict SSH source CIDR to your current public IP.
- Supply an explicitly pinned image/AMI; do not use a moving image family.
- Always run `terraform destroy` after validation.
