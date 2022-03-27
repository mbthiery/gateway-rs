use std::ops::Deref;

const RETIRED_NETID: NetId = NetId(0x200010);

#[derive(PartialEq, Clone, Copy, Debug)]
pub struct DevAddr(u32);

#[derive(PartialEq, Clone, Copy, Debug)]
pub struct SubnetAddr(u32);

#[derive(PartialEq, Clone, Copy, Debug, Default)]
pub struct NetId(u32);

#[derive(PartialEq, Debug)]
pub struct NetClass(u8);

impl From<u32> for DevAddr {
    fn from(v: u32) -> Self {
        Self(v)
    }
}

impl DevAddr {
    /// Translate from a Helium subnet address to a LoRaWAN devaddr.
    /// netid_list contains Helium's ordered list of assigned NetIDs
    ///
    pub fn from_subnet(subnetaddr: &SubnetAddr, netid_list: &[NetId]) -> Option<Self> {
        NetId::from_subnet_addr(subnetaddr, netid_list).and_then(|netid| {
            netid
                .addr_range(netid_list)
                .map(|(lower, _upper)| netid.to_devaddr(subnetaddr.0 - lower.0))
        })
    }

    /// Does this LoRaWAN devaddr belong to the Helium network?
    /// netid_list contains Helium's ordered list of assigned NetIDs
    ///
    pub fn is_local(&self, netid_list: &[NetId]) -> bool {
        NetId::from(self).is_local(netid_list)
    }

    /// Parse the LoRaWAN NetID
    ///
    pub fn net_id(&self) -> NetId {
        NetId::from(self)
    }

    fn net_class(self) -> NetClass {
        fn netid_shift_prefix(prefix: u8, index: u8) -> NetClass {
            if (prefix & (1 << index)) == 0 {
                NetClass(7 - index)
            } else if index > 0 {
                netid_shift_prefix(prefix, index - 1)
            } else {
                NetClass(0)
            }
        }

        let n_bytes = self.0.to_be_bytes();
        let first = n_bytes[0];
        netid_shift_prefix(first, 7)
    }

    fn nwk_addr(&self) -> u32 {
        let netid = NetId::from(self);
        let len = NetClass::from(&netid).addr_len();
        let mask = (1 << len) - 1;
        self.0 & mask
    }
}

impl From<u32> for SubnetAddr {
    fn from(v: u32) -> Self {
        Self(v)
    }
}

impl Deref for SubnetAddr {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl SubnetAddr {
    /// Translate from a LoRaWAN devaddr to a Helium subnet address.
    /// netid_list contains Helium's ordered list of assigned NetIDs
    ///
    pub fn from_devaddr(dev_addr: &DevAddr, netid_list: &[NetId]) -> Option<Self> {
        NetId::from(dev_addr)
            .addr_range(netid_list)
            .map(|(lower, _upper)| Self(lower.0 + dev_addr.nwk_addr()))
    }

    pub fn within_range(&self, netid: &NetId, netid_list: &[NetId]) -> bool {
        netid
            .addr_range(netid_list)
            .map_or(false, |(lower, upper)| {
                (self.0 >= lower.0) && (self.0 < upper.0)
            })
    }
}

//
// Internal functions
//
// Note - function and var names correspond closely to the LoRaWAN spec.
//

impl DevAddr {
    fn from_nwkaddr(netid: &NetId, nwkaddr: u32) -> Option<Self> {
        fn var_netid(netclass: &NetClass, addr: u32) -> u32 {
            addr << netclass.addr_len()
        }
        let netclass = NetClass::from(netid);
        let addr = netclass.var_net_class() | **netid;
        Some((var_netid(&netclass, addr) | nwkaddr).into())
    }
}

impl From<&NetId> for NetClass {
    fn from(netid: &NetId) -> Self {
        Self((netid.0 >> 21) as u8)
    }
}

impl Deref for NetClass {
    type Target = u8;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl NetClass {
    fn addr_len(&self) -> u32 {
        const ADDR_LEN: &[u8] = &[25, 24, 20, 17, 15, 13, 10, 7];
        *ADDR_LEN.get(self.0 as usize).unwrap_or(&0) as u32
    }

    fn id_len(&self) -> u32 {
        const ID_LEN: &[u8] = &[6, 6, 9, 11, 12, 13, 15, 17];
        *ID_LEN.get(self.0 as usize).unwrap_or(&0) as u32
    }

    fn var_net_class(&self) -> u32 {
        let idlen = self.id_len();
        match self.0 {
            0 => 0,
            1 => 0b10u32 << idlen,
            2 => 0b110u32 << idlen,
            3 => 0b1110u32 << idlen,
            4 => 0b11110u32 << idlen,
            5 => 0b111110u32 << idlen,
            6 => 0b1111110u32 << idlen,
            7 => 0b11111110u32 << idlen,
            _ => 0,
        }
    }
}

impl From<DevAddr> for NetId {
    fn from(dev_addr: DevAddr) -> Self {
        fn get_netid(dev_addr: &DevAddr, prefix_len: u8, nwkidbits: u32) -> u32 {
            (dev_addr.0 << (prefix_len - 1)) >> (31 - nwkidbits)
        }

        let net_type = dev_addr.net_class();
        let id = get_netid(&dev_addr, net_type.0 + 1, net_type.id_len());
        Self::from(id | ((net_type.0 as u32) << 21))
    }
}

impl From<&DevAddr> for NetId {
    fn from(dev_addr: &DevAddr) -> Self {
        Self::from(*dev_addr)
    }
}

impl From<u32> for NetId {
    fn from(v: u32) -> Self {
        Self(v & 0b111111111111111111111111)
    }
}

impl Deref for NetId {
    type Target = u32;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl NetId {
    fn is_local(&self, netid_list: &[NetId]) -> bool {
        if self == &RETIRED_NETID {
            true
        } else {
            netid_list.contains(self)
        }
    }

    fn addr_range(&self, netid_list: &[NetId]) -> Option<(SubnetAddr, SubnetAddr)> {
        // 95% of traffic is non-Helium so netid_list.contains will usually be false
        if !netid_list.contains(self) {
            return None;
        }
        let mut lower: u32 = 0;
        let mut upper: u32 = 0;
        // 5% code path
        for item in netid_list {
            let size = item.size();
            if item == self {
                upper += size;
                break;
            }
            lower += size;
            upper = lower;
        }
        Some((SubnetAddr(lower), SubnetAddr(upper)))
    }

    fn size(&self) -> u32 {
        1 << NetClass::from(self).addr_len()
    }

    fn netid_class(&self) -> NetClass {
        NetClass::from(self)
    }

    fn to_devaddr(&self, nwkaddr: u32) -> DevAddr {
        fn var_netid(netclass: &NetClass, netid: u32) -> u32 {
            netid << netclass.addr_len()
        }

        let netclass = NetClass::from(self);
        let addr = netclass.var_net_class() | self.0;
        DevAddr(var_netid(&netclass, addr) | nwkaddr)
    }

    fn from_subnet_addr(subnetaddr: &SubnetAddr, netid_list: &[NetId]) -> Option<Self> {
        netid_list
            .iter()
            .find(|item| subnetaddr.within_range(*item, netid_list))
            .cloned()
    }
}

#[cfg(test)]

mod tests {
    use super::*;
    use rand::Rng;

    fn create_netid(netclass: u8, id: u32) -> NetId {
        NetId(((netclass as u32) << 21) | id)
    }
/*
    fn mock_random_netids() -> Vec<NetId> {
        let mut rng = rand::thread_rng();
        // Len = rand:uniform(10),
        // [create_netid(rand:uniform(7), rand:uniform(64)) || _ <- lists:seq(1, Len)].
        let _len: u32 = rng.gen_range(0..10);
        let netids = (0..10)
            .map(|_| {
                let id = rng.gen_range(0..65);
                let netclass = rng.gen_range(0..8);
                create_netid(netclass, id)
            })
            .collect::<Vec<NetId>>();
        netids
    }

    fn insert_item<T>(item: T, array: &'static mut [T], pos: usize) -> &'static mut [T] {
        *array.last_mut().unwrap() = item;
        array[pos..].rotate_right(1);
        array
    }

    fn copy_slice(dst: &mut [u8], src: &[u8]) -> usize {
        dst.iter_mut().zip(src).map(|(x, y)| *x = *y).count()
    }
*/
    fn mutate_array(item: NetId, src: &[NetId], pos: usize) -> [NetId; 4] {
        let mut dst = [NetId(0), NetId(0), NetId(0), NetId(0)];
        //dst.clone_from_slice(&src[4..]);
        dst.clone_from_slice(src);
        dst[pos] = item;
        dst.clone()
    }
/*
    fn insert_rand<T>(item: T, array: &mut [T]) {
        let mut rng = rand::thread_rng();
        let alen = array.len();
        let pos: usize = rng.gen_range(0..alen) as usize;
        *array.last_mut().unwrap() = item;
        array[pos..].rotate_right(1);
    }
*/
    fn exercise_subnet_list(devaddr: DevAddr, netid_list: &[NetId]) {
        let subnet_addr = SubnetAddr::from_devaddr(&devaddr, netid_list);
        let devaddr_2 = DevAddr::from_subnet(&subnet_addr.unwrap(), netid_list);
        assert_eq!(devaddr, devaddr_2.unwrap())
    }

    fn exercise_subnet(devaddr: DevAddr) {
        let netid = NetId::from(&devaddr);
        let netid_list: [NetId; 4] = [
            NetId(0xC00050),
            NetId(0xE00001),
            NetId(0xC00035),
            NetId(0x60002D),
        ];
        exercise_subnet_list(devaddr, &mutate_array(netid, &netid_list, 0));
        exercise_subnet_list(devaddr, &mutate_array(netid, &netid_list, 1));
        exercise_subnet_list(devaddr, &mutate_array(netid, &netid_list, 2));
        exercise_subnet_list(devaddr, &mutate_array(netid, &netid_list, 3));
        ()
    }

    fn random_subnet(devaddr: &DevAddr) {
        let mut rng = rand::thread_rng();
        let _netid: NetId = devaddr.into();
        let _netids = (0..10)
            .map(|_| {
                let id = rng.gen_range(0..65);
                let netclass = rng.gen_range(0..8);
                create_netid(netclass, id)
            })
            .collect::<Vec<NetId>>();
        ()
    }

    fn addr_bit_len(devaddr: &DevAddr) -> u32 {
        let netid: NetId = devaddr.into();
        let netclass = NetClass::from(&netid);
        let addr_len = netclass.addr_len();
        addr_len
    }

    fn exercise_devaddr(netid: u32, addr: u32, _id_len: u32, _addr_len: u32) {
        let devaddr = DevAddr::from_nwkaddr(&NetId::from(netid), addr);
        let netclass = NetClass::from(&devaddr.unwrap().net_id());
        assert!(&netclass.0 <= &7);
        let netid_2 = &DevAddr::from(devaddr.unwrap().0).net_id();
        assert_eq!(netid, netid_2.0);
        // let addr_len_2 = addr_bit_len(&devaddr.unwrap());
        // assert_eq!(addr_len, addr_len_2);
        // NwkAddr = nwk_addr(DevAddr),
        // ?assertEqual(Addr, NwkAddr),
        exercise_subnet(devaddr.unwrap());
        random_subnet(&devaddr.unwrap());
        ()
    }

    fn exercise_netid(netclass: u32, id: u32, id_len: u32, addr_len: u32) {
        let netid = (netclass << 21) & id;
        //MaxNetSize = netid_size(NetID),
        exercise_devaddr(netid, 0, id_len, addr_len);
        exercise_devaddr(netid, 1, id_len, addr_len);
        exercise_devaddr(netid, 8, id_len, addr_len);
        exercise_devaddr(netid, 16, id_len, addr_len);
        exercise_devaddr(netid, 32, id_len, addr_len);
        exercise_devaddr(netid, 33, id_len, addr_len);
        exercise_devaddr(netid, 64, id_len, addr_len);
        //exercise_devaddr(netid, MaxNetSize - 1, id_len, addr_len);
    }

    #[test]
    fn test_exercise_devaddr() {
        exercise_netid(7, 2, 17, 7);
        exercise_netid(6, 2, 15, 10);
        exercise_netid(5, 2, 13, 13);
        exercise_netid(4, 2, 12, 15);
        exercise_netid(3, 2, 11, 17);
        exercise_netid(2, 2, 9, 20);
        exercise_netid(1, 2, 6, 24);
        exercise_netid(0, 2, 6, 25);
    }

    #[test]
    fn test_exercise() {
        let dev_addr_01: DevAddr = 0xFC00D410.into();
        exercise_subnet(dev_addr_01)
    }

    #[allow(non_snake_case)]
    #[test]
    fn test_net_id() {
        // LegacyDevAddr = <<$H:7, 0:25>>,
        let LegacyNetID: NetId = RETIRED_NETID;

        let NetID00: NetId = 0xE00001.into();
        let NetID01: NetId = 0xC00035.into();
        let NetID02: NetId = 0x60002D.into();
        let NetIDExt: NetId = 0xC00050.into();

        // Class 6
        let DevAddr00: DevAddr = 0x90000000.into();
        let DevAddr01: DevAddr = 0xFC00D410.into();
        let DevAddr02: DevAddr = 0xE05A0008.into();

        let NetWidth0 = NetID00.netid_class().addr_len();
        assert_eq!(7, NetWidth0);
        let NetWidth1 = NetID01.netid_class().addr_len();
        assert_eq!(10, NetWidth1);
        let NetWidth2 = NetID02.netid_class().addr_len();
        assert_eq!(17, NetWidth2);
        let NetSize0 = NetID00.size();
        assert_eq!(128, NetSize0);
        let NetSize1 = NetID01.size();
        assert_eq!(1024, NetSize1);
        let NetSize2 = NetID02.size();
        assert_eq!(131072, NetSize2);

        let NetIDList: Vec<NetId> = vec![NetID00, NetID01, NetID02];
        assert!(NetID01.is_local(&NetIDList));
        assert!(!NetIDExt.is_local(&NetIDList));
        assert!(LegacyNetID.is_local(&NetIDList));

        let DevAddrLegacy = DevAddr::from_nwkaddr(&LegacyNetID, 0).expect("dev_addr");
        assert_eq!(DevAddr00, DevAddrLegacy);
        let DevAddr1 = DevAddr::from_nwkaddr(&NetID01, 16).expect("dev_addr");
        assert_eq!(DevAddr01, DevAddr1);
        let DevAddr2 = DevAddr::from_nwkaddr(&NetID02, 8).expect("dev_addr");
        assert_eq!(DevAddr02, DevAddr2);

        let NetIDType00 = DevAddr00.net_class();
        assert_eq!(1, *NetIDType00);
        let NetIDType01 = DevAddr01.net_class();
        assert_eq!(6, *NetIDType01);
        let NetIDType02 = DevAddr02.net_class();
        assert_eq!(3, *NetIDType02);

        let NetIDType0 = DevAddrLegacy.net_class();
        assert_eq!(1, *NetIDType0);
        let NetIDType1 = DevAddr1.net_class();
        assert_eq!(6, *NetIDType1);
        let NetIDType2 = DevAddr2.net_class();
        assert_eq!(3, *NetIDType2);

        let NetIDType0 = DevAddrLegacy.net_class();
        assert_eq!(1, *NetIDType0);
        let NetIDType1 = DevAddr1.net_class();
        assert_eq!(6, *NetIDType1);
        let NetIDType2 = DevAddr2.net_class();
        assert_eq!(3, *NetIDType2);

        let NetID_0: NetId = DevAddr00.into();
        assert_eq!(NetID_0, LegacyNetID);
        //let NetID_1_a = parse_netid(0xFC00D410);
        //assert_eq!(NetID_1_a, 0xC00035);
        let NetID_1: NetId = DevAddr01.into();
        assert_eq!(NetID_1, NetID01);
        let NetID_2: NetId = DevAddr02.into();
        assert_eq!(NetID_2, NetID02);

        let NetID0: NetId = DevAddrLegacy.into();
        assert_eq!(NetID0, LegacyNetID);
        let NetID1: NetId = DevAddr1.into();
        assert_eq!(NetID1, NetID01);
        let NetID2: NetId = DevAddr2.into();
        assert_eq!(NetID2, NetID02);

        let Width_0 = addr_bit_len(&DevAddr00);
        assert_eq!(24, Width_0);
        let Width_1 = addr_bit_len(&DevAddr01);
        assert_eq!(10, Width_1);
        let Width_2 = addr_bit_len(&DevAddr02);
        assert_eq!(17, Width_2);

        let Width0 = addr_bit_len(&DevAddrLegacy);
        assert_eq!(24, Width0);
        let Width1 = addr_bit_len(&DevAddr1);
        assert_eq!(10, Width1);
        let Width2 = addr_bit_len(&DevAddr2);
        assert_eq!(17, Width2);

        let NwkAddr0 = DevAddr00.nwk_addr();
        assert_eq!(0, NwkAddr0);
        let NwkAddr1 = DevAddr01.nwk_addr();
        assert_eq!(16, NwkAddr1);
        let NwkAddr2 = DevAddr02.nwk_addr();
        assert_eq!(8, NwkAddr2);

        // Backwards DevAddr compatibility test
        // DevAddr00 is a legacy Helium Devaddr.  The NetID is retired.
        // By design we do compute a proper subnet (giving us a correct OUI route),
        // but if we compute the associated DevAddr for this subnet (for the Join request)
        // we'll get a new one associated with a current and proper NetID
        // In other words, DevAddr00 is not equal to DevAddr000.
        let Subnet0 = SubnetAddr::from_devaddr(&DevAddr00, &NetIDList);
        assert_eq!(None, Subnet0);
        let SubnetZero: SubnetAddr = 0x0.into();
        let DevAddr000 = DevAddr::from_subnet(&SubnetZero, &NetIDList).expect("dev_addr");
        // By design the reverse DevAddr will have a correct NetID
        assert_ne!(DevAddr000, DevAddr00);
        // FixMe assert_eq!(Some(0xFE000080), DevAddr000.unwrap());
        let DevAddr000NetID = NetId::from(DevAddr000);
        assert_eq!(NetID00, DevAddr000NetID);

        let Subnet1 = SubnetAddr::from_devaddr(&DevAddr01, &NetIDList).expect("subnet_addr");
        assert_eq!((1 << 7) + 16, *Subnet1);
        let DevAddr001 = DevAddr::from_subnet(&Subnet1, &NetIDList).expect("dev_addr");
        assert_eq!(DevAddr001, DevAddr01);

        let Subnet1 = SubnetAddr::from_devaddr(&DevAddr01, &NetIDList).expect("subnet_addr");
        assert_eq!((1 << 7) + 16, *Subnet1);
        let DevAddr001 = DevAddr::from_subnet(&Subnet1, &NetIDList).expect("dev_addr");
        assert_eq!(DevAddr001, DevAddr01);

        let Subnet2 = SubnetAddr::from_devaddr(&DevAddr02, &NetIDList).expect("subnet_addr");
        assert_eq!((1 << 7) + (1 << 10) + 8, *Subnet2);
        let DevAddr002 = DevAddr::from_subnet(&Subnet2, &NetIDList).expect("subnet_addr");
        assert_eq!(DevAddr002, DevAddr02);
    }

    #[test]
    fn test_id() {
        // CP data (matches Erlang test cases)
        // <<91, 255, 255, 255>> "[45] == 2D == 45 type 0"
        assert_eq!(NetId::from(0x00002D), DevAddr::from(0x5BFFFFFF).net_id());
        // <<173, 255, 255, 255>> "[45] == 2D == 45 type 1"
        assert_eq!(NetId::from(0x20002D), DevAddr::from(0xADFFFFFF).net_id());
        // <<214, 223, 255, 255>> "[1,109] == 16D == 365 type 2"
        assert_eq!(NetId::from(0x40016D), DevAddr::from(0xD6DFFFFF).net_id());
        // <<235, 111, 255, 255>>), "[5,183] == 5B7 == 1463 type 3"
        assert_eq!(NetId::from(0x6005B7), DevAddr::from(0xEB6FFFFF).net_id());
        // <<245, 182, 255, 255>>), "[11, 109] == B6D == 2925 type 4"
        assert_eq!(NetId::from(0x800B6D), DevAddr::from(0xF5B6FFFF).net_id());
        // <<250, 219, 127, 255>>), "[22,219] == 16DB == 5851 type 5"
        assert_eq!(NetId::from(0xA016DB), DevAddr::from(0xFADB7FFF).net_id());
        // <<253, 109, 183, 255>> "[91, 109] == 5B6D == 23405 type 6"
        assert_eq!(NetId::from(0xC05B6D), DevAddr::from(0xFD6DB7FF).net_id());
        // <<254, 182, 219, 127>> "[1,109,182] == 16DB6 == 93622 type 7"
        assert_eq!(NetId::from(0xE16DB6), DevAddr::from(0xFEB6DB7F).net_id());
        println!(
            "left: {:#04X?} right: {:#04X?}",
            NetId::from(0xA016DB),
            NetId::from(0xFFFFFFFF)
        );
        // FixME - Invalid NetID type
        assert_eq!(NetId::from(127), DevAddr::from(0xFFFFFFFF).net_id());

        // Actility spreadsheet examples
        assert_eq!(NetId::from(0), DevAddr::from(0).net_id());
        assert_eq!(NetId::from(1), DevAddr::from(1 << 25).net_id());
        assert_eq!(NetId::from(2), DevAddr::from(1 << 26).net_id());

        // Mis-parsed as netid 4 of type 3
        assert_eq!(NetId::from(0x600004), DevAddr::from(0xE009ABCD).net_id());
        // Valid DevAddr, NetID not assigned
        assert_eq!(NetId::from(0x20002D), DevAddr::from(0xADFFFFFF).net_id());
        // Less than 32 bit number
        assert_eq!(NetId::from(0), DevAddr::from(46377).net_id());

        // Louis test data
        assert_eq!(NetId::from(0x600002), DevAddr::from(0xE0040001).net_id());
        assert_eq!(NetId::from(0x600002), DevAddr::from(0xE0052784).net_id());
        assert_eq!(NetId::from(0x000002), DevAddr::from(0x0410BEA3).net_id());
    }
}
