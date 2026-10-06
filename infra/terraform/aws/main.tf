terraform {
  required_version = ">= 1.5.0"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "6.67.0"
    }
  }
}

provider "aws" {
  region = var.aws_region
}

variable "aws_region" {
  type    = string
  default = "us-east-1"
}

variable "instance_type" {
  type    = string
  default = "t3.small"
}

variable "repo_url" {
  type        = string
  description = "Git URL for the AgentSnap repository"
}

variable "my_ip_cidr" {
  type        = string
  description = "Your IP in CIDR form, e.g. 203.0.113.10/32"
}

variable "ami_id" {
  type        = string
  description = "Pinned Ubuntu 24.04 amd64 AMI ID for aws_region"
}

resource "aws_vpc" "demo" {
  cidr_block           = "10.77.0.0/16"
  enable_dns_support   = true
  enable_dns_hostnames = true

  tags = {
    Name = "agentsnap-demo-vpc"
  }
}

resource "aws_subnet" "demo" {
  vpc_id                  = aws_vpc.demo.id
  cidr_block              = "10.77.1.0/24"
  map_public_ip_on_launch = true

  tags = {
    Name = "agentsnap-demo-subnet"
  }
}

resource "aws_internet_gateway" "demo" {
  vpc_id = aws_vpc.demo.id

  tags = {
    Name = "agentsnap-demo-igw"
  }
}

resource "aws_route_table" "demo" {
  vpc_id = aws_vpc.demo.id

  route {
    cidr_block = "0.0.0.0/0"
    gateway_id = aws_internet_gateway.demo.id
  }

  tags = {
    Name = "agentsnap-demo-rt"
  }
}

resource "aws_route_table_association" "demo" {
  subnet_id      = aws_subnet.demo.id
  route_table_id = aws_route_table.demo.id
}

resource "aws_security_group" "demo" {
  name        = "agentsnap-demo-sg"
  description = "Lock down SSH to a single source IP"
  vpc_id      = aws_vpc.demo.id

  ingress {
    from_port   = 22
    to_port     = 22
    protocol    = "tcp"
    cidr_blocks = [var.my_ip_cidr]
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = {
    Name = "agentsnap-demo-sg"
  }
}

resource "aws_instance" "demo" {
  ami                    = var.ami_id
  instance_type          = var.instance_type
  subnet_id              = aws_subnet.demo.id
  vpc_security_group_ids = [aws_security_group.demo.id]

  metadata_options {
    http_endpoint               = "enabled"
    http_tokens                 = "required"
    http_put_response_hop_limit = 1
    instance_metadata_tags      = "disabled"
  }

  root_block_device {
    encrypted = true
  }

  user_data = templatefile("${path.module}/cloud-init.sh", {
    repo_url = var.repo_url
  })

  tags = {
    Name = "agentsnap-disposable-demo"
  }
}

output "public_ip" {
  description = "Public IP for SSH access"
  value       = aws_instance.demo.public_ip
}
