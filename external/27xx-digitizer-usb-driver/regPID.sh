#!/bin/bash
ip addr show > /tmp/tmp.file
string=$(grep fda7:cae0 /tmp/tmp.file)
if [ -z "$string" ]
then
echo "CAEN Digitizer not found"
exit
fi
search="/64"
temp=${string%$search*}
search1="inet6"
ip=${temp#*$search1}

iparray=(${ip//:/ })
hostip=$(echo ${iparray[0]}:${iparray[1]}:${iparray[2]}:${iparray[3]})::1
high=$(echo 0x${iparray[2]})
snH=$(echo "$((high << 16 ))")
snL=$(printf "%d" 0x${iparray[3]})
sn=$((snH + snL))

echo "$hostip   CAENDGTZ-USB-$sn CAENDGTZ-USB-$sn.local"        >> /etc/hosts
echo "CAENDGTZ-USB-$sn registered"
