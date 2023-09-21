data "google_compute_regions" "available" {}

data "google_compute_zones" "available" {
  for_each = toset(data.google_compute_regions.available.names)
  region   = each.value
}

data "google_compute_image" "ubuntu" {
  family  = "ubuntu-2204-lts"
  project = "ubuntu-os-cloud"
}
