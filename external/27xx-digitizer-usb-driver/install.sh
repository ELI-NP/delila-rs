#!/bin/bash

# Set MTU to 15000 on CDC
## First version
echo "SUBSYSTEM==\"usb\", ATTRS{idVendor}==\"21e1\", ATTRS{idProduct}==\"0018\",  ACTION==\"add\", ENV{CAEN_DEVICE}=\"dig2\"" > /etc/udev/rules.d/90-CAEN-DIG2.rules
echo "IMPORT{parent}=\"CAEN_DEVICE\"" >> /etc/udev/rules.d/90-CAEN-DIG2.rules
echo "SUBSYSTEM==\"net\", ACTION==\"add\", DRIVERS==\"cdc_eem\", ENV{CAEN_DEVICE}==\"dig2\", RUN+=\"/sbin/ip link set mtu 15000 dev %k\"" >> /etc/udev/rules.d/90-CAEN-DIG2.rules
echo "SUBSYSTEM==\"net\", ACTION==\"move\", DRIVERS==\"cdc_eem\", ENV{CAEN_DEVICE}==\"dig2\", RUN+=\"/sbin/ip link set mtu 15000 dev %k\"" >> /etc/udev/rules.d/90-CAEN-DIG2.rules

## Second version, working at least on Ubuntu 24.04, appended just after the first version
echo "SUBSYSTEM==\"net\", ACTION==\"add\", ENV{ID_NET_DRIVER}==\"cdc_eem\", ENV{ID_USB_VENDOR_ID}==\"21e1\", ENV{ID_USB_MODEL_ID}==\"0018\", RUN+=\"/sbin/ip link set mtu 15000 dev %k\"" >> /etc/udev/rules.d/90-CAEN-DIG2.rules
echo "SUBSYSTEM==\"net\", ACTION==\"move\", ENV{ID_NET_DRIVER}==\"cdc_eem\", ENV{ID_USB_VENDOR_ID}==\"21e1\", ENV{ID_USB_MODEL_ID}==\"0018\", RUN+=\"/sbin/ip link set mtu 15000 dev %k\"" >> /etc/udev/rules.d/90-CAEN-DIG2.rules

udevadm control --reload-rules

start="hosts:"
search="mdns4_minimal"
replace="mdns_minimal"
sed -i -e "/^$start/s/$search/$replace/g" /etc/nsswitch.conf

echo "Driver CAENDGTZ-USB installed!"
