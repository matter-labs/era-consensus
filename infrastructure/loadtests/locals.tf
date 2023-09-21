locals {
  zones_by_region = {
    for region, zones in data.google_compute_zones.available : region => zones.names
  }

  all_regions    = keys(local.zones_by_region)
  random_regions = random_shuffle.region.result

  instances_distribution = [
    for idx in range(var.num_instances) : {
      region = local.random_regions[idx % length(local.random_regions)]
      zone   = random_shuffle.zone[local.random_regions[idx % length(local.random_regions)]].result[0]
    }
  ]

  network = "default"
}
