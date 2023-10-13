output "instance_public_ips" {
  description = "Public IPs of the instances"
  value       = [for instance in google_compute_instance.zksync_bft_node : instance.network_interface[0].access_config[0].nat_ip]
}

output "ssh_key" {
  value     = tls_private_key.ssh.public_key_openssh
  sensitive = true
}
