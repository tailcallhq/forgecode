variable "environment" {
  description = "The deployment environment (e.g., dev, staging, production)"
  type        = string
  default     = "production"
}

variable "region" {
  description = "The AWS region to deploy resources into"
  type        = string
}

variable "vpc_cidr" {
  description = "The CIDR block for the VPC"
  type        = string
}

variable "service_name" {
  description = "The name of the ECS service"
  type        = string
  default     = "forgecode-service"
}

variable "container_image" {
  description = "The Docker image to deploy"
  type        = string
}

variable "container_port" {
  description = "The port the container listens on"
  type        = number
  default     = 8080
}

variable "db_instance_class" {
  description = "The RDS instance class"
  type        = string
  default     = "db.t3.micro"
}

variable "db_password" {
  description = "The master password for the RDS instance"
  type        = string
  sensitive   = true
}

variable "s3_bucket_name" {
  description = "The name of the S3 bucket for application data"
  type        = string
}

variable "cloudfront_price_class" {
  description = "The CloudFront price class"
  type        = string
  default     = "PriceClass_100"
}
