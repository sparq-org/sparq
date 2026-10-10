# Enclave image for sparq-vcq-tee. run.sh builds the binary on the parent
# instance (Amazon Linux 2023) and copies it in, so the base matches its glibc.
FROM amazonlinux:2023
COPY sparq-vcq-tee-enclave /usr/local/bin/sparq-vcq-tee-enclave
CMD ["/usr/local/bin/sparq-vcq-tee-enclave"]
