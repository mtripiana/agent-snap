terraform {
  required_version = ">= 1.5.0"
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "8.4.0"
    }
  }
}

provider "google" {
  project = var.project_id
  region  = var.region
  zone    = var.zone
}

variable "project_id" {
  type        = string
  description = "GCP project id"
}

variable "region" {
  type    = string
  default = "us-central1"
}

variable "zone" {
  type    = string
  default = "us-central1-a"
}

variable "machine_type" {
  type    = string
  default = "e2-standard-2"
}

variable "repo_url" {
  type        = string
  description = "Git URL for the AgentSnap repository"
}

variable "my_ip_cidr" {
  type        = string
  description = "Your public IP CIDR, e.g. 203.0.113.10/32"
}

variable "image" {
  type        = string
  description = "Pinned Ubuntu 24.04 amd64 image self-link or project/name"
}

resource "google_compute_network" "demo" {
  name                    = "agentsnap-demo-network"
  auto_create_subnetworks = false
}

resource "google_compute_subnetwork" "demo" {
  name          = "agentsnap-demo-subnet"
  ip_cidr_range = "10.88.1.0/24"
  region        = var.region
  network       = google_compute_network.demo.id
}

resource "google_compute_firewall" "ssh" {
  name    = "agentsnap-demo-allow-ssh"
  network = google_compute_network.demo.name

  allow {
    protocol = "tcp"
    ports    = ["22"]
  }

  source_ranges = [var.my_ip_cidr]
  target_tags   = ["agentsnap-demo"]
}

resource "google_compute_instance" "demo" {
  name         = "agentsnap-disposable-demo"
  machine_type = var.machine_type
  zone         = var.zone
  tags         = ["agentsnap-demo"]

  boot_disk {
    initialize_params {
      image = var.image
      size  = 30
      type  = "pd-balanced"
    }
  }

  network_interface {
    subnetwork = google_compute_subnetwork.demo.id

    access_config {}
  }

  service_account {
    scopes = ["https://www.googleapis.com/auth/logging.write"]
  }

  metadata_startup_script = templatefile("${path.module}/startup.sh", {
    repo_url = var.repo_url
  })

}

output "public_ip" {
  description = "Public IP for SSH access"
  value       = google_compute_instance.demo.network_interface[0].access_config[0].nat_ip
}
