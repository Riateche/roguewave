# Use an official Ubuntu base image
FROM ubuntu:24.04

# Set environment variables to avoid interactive prompts during installation
ENV DEBIAN_FRONTEND=noninteractive

# Install OpenSSH server and clean up
# Create the privilege separation directory and fix permissions
RUN apt-get update && \
    apt-get install -y openssh-server iputils-ping telnet iproute2 sudo && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/* && \
    mkdir -p /run/sshd && \
    chmod 755 /run/sshd

# Set up SSH configuration
COPY ci_ssh_key.pub /root/.ssh/authorized_keys
RUN chmod 600 /root/.ssh/authorized_keys

# Expose SSH port
EXPOSE 22

# Start SSH server
CMD ["/usr/sbin/sshd", "-D"]
