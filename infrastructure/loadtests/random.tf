resource "random_shuffle" "region" {
  input        = local.all_regions
  result_count = length(local.all_regions)
  seed = "jifwoeofcnvkjkdshfa"
}

resource "random_shuffle" "zone" {
  for_each = local.zones_by_region

  input        = each.value
  result_count = 1
  seed = "dowoweff;manerdf"
}
