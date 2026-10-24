#!/bin/bash
# The FT232R wedges when the phone resets; rebind it and restore settings.
P=$(basename "$(readlink -f /sys/bus/usb-serial/devices/ttyUSB0/../..)")
sudo sh -c "echo $P > /sys/bus/usb/drivers/usb/unbind; sleep 1; echo $P > /sys/bus/usb/drivers/usb/bind; sleep 1; echo 1 > /sys/bus/usb-serial/devices/ttyUSB0/latency_timer"
stty -F /dev/ttyUSB0 921600 raw -echo -crtscts cs8 -cstopb -parenb clocal
