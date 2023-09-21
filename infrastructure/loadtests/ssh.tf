resource "tls_private_key" "ssh" {
  algorithm = "RSA"
  rsa_bits  = 4096
}

resource "local_file" "ssh_private_key_pem" {
  content         = tls_private_key.ssh.private_key_pem
  filename        = "ansible/.ssh/google_compute_engine"
  file_permission = "0600"
}

resource "google_os_login_ssh_public_key" "cache" {
  user = data.google_client_openid_userinfo.me.email
  key  = tls_private_key.ssh.public_key_openssh
}


data "google_client_openid_userinfo" "me" {}
