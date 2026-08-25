.PHONY: help \
	test lint package \
	demo-aws-init demo-aws-apply demo-aws-destroy \
	demo-gcp-init demo-gcp-apply demo-gcp-destroy

help:
	@echo "AgentSnap helper targets"
	@echo ""
	@echo "  make test"
	@echo "  make lint"
	@echo "  make package"
	@echo "AWS Terraform demo"
	@echo "  make demo-aws-init"
	@echo "  make demo-aws-apply REPO_URL=... MY_IP_CIDR=... AMI_ID=... [AWS_REGION=us-east-1]"
	@echo "  make demo-aws-destroy REPO_URL=... MY_IP_CIDR=... AMI_ID=... [AWS_REGION=us-east-1]"
	@echo ""
	@echo "GCP Terraform demo"
	@echo "  make demo-gcp-init"
	@echo "  make demo-gcp-apply PROJECT_ID=... REPO_URL=... MY_IP_CIDR=... IMAGE=... [REGION=us-central1] [ZONE=us-central1-a]"
	@echo "  make demo-gcp-destroy PROJECT_ID=... REPO_URL=... MY_IP_CIDR=... IMAGE=... [REGION=us-central1] [ZONE=us-central1-a]"

AWS_REGION ?= us-east-1
REGION ?= us-central1
ZONE ?= us-central1-a

test:
	./scripts/test.sh

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
	terraform fmt -check -recursive

package:
	cargo package -p agentsnap-core --locked
	(cd sdks/python && python3 -m build)
	(cd sdks/node && npm pack --dry-run)

demo-aws-init:
	cd infra/terraform/aws && terraform init

demo-aws-apply:
	@test -n "$(REPO_URL)" || (echo "REPO_URL is required" && exit 1)
	@test -n "$(MY_IP_CIDR)" || (echo "MY_IP_CIDR is required" && exit 1)
	@test -n "$(AMI_ID)" || (echo "AMI_ID is required (use a pinned Ubuntu 24.04 AMI for AWS_REGION)" && exit 1)
	cd infra/terraform/aws && terraform apply \
		-var="repo_url=$(REPO_URL)" \
		-var="my_ip_cidr=$(MY_IP_CIDR)" \
		-var="ami_id=$(AMI_ID)" \
		-var="aws_region=$(AWS_REGION)"

demo-aws-destroy:
	@test -n "$(REPO_URL)" || (echo "REPO_URL is required" && exit 1)
	@test -n "$(MY_IP_CIDR)" || (echo "MY_IP_CIDR is required" && exit 1)
	@test -n "$(AMI_ID)" || (echo "AMI_ID is required (use a pinned Ubuntu 24.04 AMI for AWS_REGION)" && exit 1)
	cd infra/terraform/aws && terraform destroy \
		-var="repo_url=$(REPO_URL)" \
		-var="my_ip_cidr=$(MY_IP_CIDR)" \
		-var="ami_id=$(AMI_ID)" \
		-var="aws_region=$(AWS_REGION)"

demo-gcp-init:
	cd infra/terraform/gcp && terraform init

demo-gcp-apply:
	@test -n "$(PROJECT_ID)" || (echo "PROJECT_ID is required" && exit 1)
	@test -n "$(REPO_URL)" || (echo "REPO_URL is required" && exit 1)
	@test -n "$(MY_IP_CIDR)" || (echo "MY_IP_CIDR is required" && exit 1)
	@test -n "$(IMAGE)" || (echo "IMAGE is required (use a pinned Ubuntu 24.04 image)" && exit 1)
	cd infra/terraform/gcp && terraform apply \
		-var="project_id=$(PROJECT_ID)" \
		-var="repo_url=$(REPO_URL)" \
		-var="my_ip_cidr=$(MY_IP_CIDR)" \
		-var="image=$(IMAGE)" \
		-var="region=$(REGION)" \
		-var="zone=$(ZONE)"

demo-gcp-destroy:
	@test -n "$(PROJECT_ID)" || (echo "PROJECT_ID is required" && exit 1)
	@test -n "$(REPO_URL)" || (echo "REPO_URL is required" && exit 1)
	@test -n "$(MY_IP_CIDR)" || (echo "MY_IP_CIDR is required" && exit 1)
	@test -n "$(IMAGE)" || (echo "IMAGE is required (use a pinned Ubuntu 24.04 image)" && exit 1)
	cd infra/terraform/gcp && terraform destroy \
		-var="project_id=$(PROJECT_ID)" \
		-var="repo_url=$(REPO_URL)" \
		-var="my_ip_cidr=$(MY_IP_CIDR)" \
		-var="image=$(IMAGE)" \
		-var="region=$(REGION)" \
		-var="zone=$(ZONE)"
