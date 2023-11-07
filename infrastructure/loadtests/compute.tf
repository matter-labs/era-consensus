resource "google_compute_firewall" "allow_zksync_bft_node_port" {
  name    = "allow-zksync-bft-node-port-${var.test_id}"
  network = local.network

  allow {
    protocol = "tcp"
    ports    = [var.node_port]
  }

  target_tags   = ["allow-zksync-bft-node-port"]
  source_ranges = [for instance in google_compute_instance.zksync_bft_node : "${instance.network_interface[0].access_config[0].nat_ip}/32"]
}

resource "google_compute_firewall" "allow_zksync_bft_metrics_port" {
  name    = "allow-zksync-bft-metrics-port-${var.test_id}"
  network = local.network

  allow {
    protocol = "tcp"
    ports    = [var.metrics_port, "9100"]
  }

  target_tags   = ["allow-zksync-bft-metrics-port"]
  source_ranges = ["${google_compute_instance.vmagent.network_interface[0].access_config[0].nat_ip}/32"]
}

resource "google_compute_instance" "zksync_bft_node" {
  count        = var.num_instances
  name         = "zksync-bft-loadtest-${var.test_id}-${count.index}-${local.instances_distribution[count.index].zone}"
  machine_type = "e2-highcpu-8"
  zone         = local.instances_distribution[count.index].zone

  metadata = {
    enable-oslogin : "TRUE"
  }

  tags = ["allow-zksync-bft-node-port", "allow-zksync-bft-metrics-port"]

  labels = {
    repo    = "zksync-bft"
    purpose = "loadtest"
    test_id = var.test_id
  }

  boot_disk {
    initialize_params {
      image = data.google_compute_image.ubuntu.self_link
    }
  }

  network_interface {
    network = local.network
    access_config {}
  }

  lifecycle {
    precondition {
      condition     = var.node_port != var.metrics_port
      error_message = "Node port and metrics port should not be equal"
    }
  }
}

resource "google_compute_instance" "vmagent" {
  name         = "zksync-bft-loadtest-${var.test_id}-vmagent"
  machine_type = "e2-highcpu-4"
  zone         = "us-central1-a"

  labels = {
    repo    = "zksync-bft"
    purpose = "monitoring"
    test_id = var.test_id
  }

  boot_disk {
    initialize_params {
      image = data.google_compute_image.ubuntu.self_link
    }
  }

  network_interface {
    network = "default"
    access_config {
    }
  }

  service_account {
    # Google recommends custom service accounts that have cloud-platform scope and permissions granted via IAM Roles.
    email  = module.vmagent_sa.email
    scopes = ["compute-ro"]
  }
}

module "vmagent_sa" {
  source      = "terraform-google-modules/service-accounts/google"
  version     = "4.2.1"
  project_id  = var.project_id
  description = "zksync-bft-loadtest-vmagent-${var.test_id}"
  prefix      = "zksync-bft-vmagent"
  names       = [var.test_id]
  project_roles = [
    "${var.project_id}=>roles/compute.viewer",
  ]
}
