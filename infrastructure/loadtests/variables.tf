variable "project_id" {
  default = "matterlabs-gpu-sandbox"
}

variable "test_id" {
  type = string

  validation {
    condition     = var.test_id != ""
    error_message = "Unique test ID should not be empty"
  }
}

variable "num_instances" {
  type = number

  validation {
    condition     = var.num_instances >= 1 && var.num_instances <= 200
    error_message = "Number of instances should be more than 0 and not exceed 200"
  }
}

variable "node_port" {
  type        = number
  description = "Port to be exposed by VMs and used in node config"

  validation {
    condition     = var.node_port >= 1024 && var.node_port <= 65535
    error_message = "Port should be not empty number and be unprivileged (1024 to 65535)"
  }
}

variable "metrics_port" {
  type        = number
  description = "Prometheus metrics port to be exposed by VMs and used in node config"

  validation {
    condition     = var.metrics_port >= 1024 && var.metrics_port <= 65535
    error_message = "Port should be not empty number and be unprivileged (1024 to 65535)"
  }
}
