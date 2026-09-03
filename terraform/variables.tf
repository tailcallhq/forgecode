variable "environment" {
  description = "The deployment environment (e.g., dev, staging, production)"
  type        = string
}

variable "container_image" {
  description = "The Docker image to deploy"
  type        = string
}

variable "s3_bucket_name" {
  description = "The name of the S3 bucket for application data"
  type        = string
}

variable "db_password" {
  description = "The master password for the RDS instances"
  type        = string
  sensitive   = true
}
