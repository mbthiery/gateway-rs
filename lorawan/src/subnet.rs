const RETIRED_NETID: NetId = NetId(0x200010);

#[derive(PartialEq, Clone, Copy, Debug)]
pub struct DevAddr(u32);

#[derive(PartialEq, Debug)]
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

impl From<&NetId> for NetClass {
    fn from(netid: &NetId) -> Self {
        Self((netid.0 >> 21) as u8)
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
}

impl From<&DevAddr> for NetId {
    fn from(dev_addr: &DevAddr) -> Self {
        fn get_netid(dev_addr: &DevAddr, prefix_len: u8, nwkidbits: u32) -> u32 {
            (dev_addr.0 << (prefix_len - 1)) >> (31 - nwkidbits)
        }

        let net_type = dev_addr.net_class();
        let id = get_netid(dev_addr, net_type.0 + 1, net_type.id_len());
        Self(id | ((net_type.0 as u32) << 21))
    }
}

impl From<u32> for NetId {
    fn from(v: u32) -> Self {
        Self(v)
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

    fn to_devaddr(&self, nwkaddr: u32) -> DevAddr {
        fn var_net_class(netclass: &NetClass) -> u32 {
            let idlen = netclass.id_len();
            match netclass.0 {
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

        fn var_netid(netclass: &NetClass, netid: u32) -> u32 {
            netid << netclass.addr_len()
        }

        let netclass = NetClass::from(self);
        let id = self.0 & 0b111111111111111111111;
        let addr = var_net_class(&netclass) | id;
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

static NETID_LIST: [NetId; 4] = [NetId(0xC00050), NetId(0xE00001), NetId(0xC00035), NetId(0x60002D)];

mod tests {
    use super::*;
    use rand::Rng;

    fn create_netid(netclass: u8, id: u32) -> NetId {
        NetId(((netclass as u32) << 21) | id)
    }

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

    fn mutate_array(item: NetId, src: &[NetId], pos: usize) -> [NetId; 4] {
        let mut dst = [NetId(0), NetId(0), NetId(0), NetId(0)];
        //println!("src is: {:#04X?}", src);
        //dst.clone_from_slice(&src[4..]);
        dst.clone_from_slice(src);
        dst[pos] = item;
        //println!("src is: {:#04X?}", src);
        //println!("dst is: {:#04X?}", dst);
        dst.clone()
    }

    fn insert_rand<T>(item: T, array: &mut [T]) {
        let mut rng = rand::thread_rng();
        let alen = array.len();
        let pos: usize = rng.gen_range(0..alen) as usize;
        *array.last_mut().unwrap() = item;
        array[pos..].rotate_right(1);
    }

    fn exercise_subnet_list(_devaddr: DevAddr, _netid_list: &[NetId]) {
        //let subnet_addr = subnet_from_devaddr(devaddr, netid_list);
        // let subnet_addr = SubnetAddr::from_devaddr(&devaddr, netid_list);
        //let devaddr_2 = devaddr_from_subnet(subnet_addr.unwrap(), netid_list);
        // let devaddr_2 = DevAddr::from_subnet(Some(&subnet_addr), netid_list);
        //assert_eq!(devaddr, devaddr_2);
        ()
    }

    fn exercise_subnet(devaddr: DevAddr) {
        let netid = NetId::from(&devaddr);
        let netid_list: [NetId; 4] = [NetId(0xC00050), NetId(0xE00001), NetId(0xC00035), NetId(0x60002D)];
        exercise_subnet_list(devaddr, &mutate_array(netid, &netid_list, 0));
        exercise_subnet_list(devaddr, &mutate_array(netid, &netid_list, 1));
        exercise_subnet_list(devaddr, &mutate_array(netid, &netid_list, 2));
        exercise_subnet_list(devaddr, &mutate_array(netid, &netid_list, 3));
        ()
    }

    fn addr_bit_len(_devaddr: u32) -> u32 {
        0
        //let netid = parse_netid(devaddr);
        //addr_len(netid_class(netid))
    }

    #[test]
    fn test_exercise() {
        let netid_1 = create_netid(0x2, 123) as u32;
        println!("NetID_1_a is: {:#04X?}", netid_1);
        let netids = mock_random_netids();
        println!("devaddr is: {:#04X?}", 123);
        println!("netids is: {:#04X?}", netids);
        assert_eq!(7, 7);
        let dev_addr_01: DevAddr = 0xFC00D410;
        exercise_subnet(dev_addr_01)
    }

    #[allow(non_snake_case)]
    #[test]
    fn test_netid() {
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

        let NetWidth0 = addr_len(netid_class(NetID00));
        assert_eq!(7, NetWidth0);
        let NetWidth1 = addr_len(netid_class(NetID01));
        assert_eq!(10, NetWidth1);
        let NetWidth2 = addr_len(netid_class(NetID02));
        assert_eq!(17, NetWidth2);
        let NetSize0 = netid_size(NetID00);
        assert_eq!(128, NetSize0);
        let NetSize1 = netid_size(NetID01);
        assert_eq!(1024, NetSize1);
        let NetSize2 = netid_size(NetID02);
        assert_eq!(131072, NetSize2);

        let NetIDList: Vec<NetId> = vec![NetID00, NetID01, NetID02];
        assert!(NetID01.is_local(&NetIDList));
        assert!(!NetIDExt.is_local(&NetIDList));
        assert!(LegacyNetID.is_local(&NetIDList));

        let DevAddrLegacy = devaddr(LegacyNetID, 0);
        assert_eq!(DevAddr00, DevAddrLegacy);
        let DevAddr1 = devaddr(NetID01, 16);
        assert_eq!(DevAddr01, DevAddr1);
        let DevAddr2 = devaddr(NetID02, 8);
        assert_eq!(DevAddr02, DevAddr2);

        let NetIDType00 = netid_type(DevAddr00);
        assert_eq!(1, NetIDType00);
        let NetIDType01 = netid_type(DevAddr01);
        assert_eq!(6, NetIDType01);
        let NetIDType02 = netid_type(DevAddr02);
        assert_eq!(3, NetIDType02);

        let NetIDType0 = netid_type(DevAddrLegacy);
        assert_eq!(1, NetIDType0);
        let NetIDType1 = netid_type(DevAddr1);
        assert_eq!(6, NetIDType1);
        let NetIDType2 = netid_type(DevAddr2);
        assert_eq!(3, NetIDType2);

        let NetIDType0 = netid_type(DevAddrLegacy);
        assert_eq!(1, NetIDType0);
        let NetIDType1 = netid_type(DevAddr1);
        assert_eq!(6, NetIDType1);
        let NetIDType2 = netid_type(DevAddr2);
        assert_eq!(3, NetIDType2);

        let NetID_0 = parse_netid(DevAddr00);
        assert_eq!(NetID_0, LegacyNetID);
        let NetID_1_a = parse_netid(0xFC00D410);
        assert_eq!(NetID_1_a, 0xC00035);
        let NetID_1 = parse_netid(DevAddr01);
        assert_eq!(NetID_1, NetID01);
        let NetID_2 = parse_netid(DevAddr02);
        assert_eq!(NetID_2, NetID02);

        let NetID0 = parse_netid(DevAddrLegacy);
        assert_eq!(NetID0, LegacyNetID);
        let NetID1 = parse_netid(DevAddr1);
        assert_eq!(NetID1, NetID01);
        let NetID2 = parse_netid(DevAddr2);
        assert_eq!(NetID2, NetID02);

        let Width_0 = addr_bit_len(DevAddr00);
        assert_eq!(24, Width_0);
        let Width_1 = addr_bit_len(DevAddr01);
        assert_eq!(10, Width_1);
        let Width_2 = addr_bit_len(DevAddr02);
        assert_eq!(17, Width_2);

        let Width0 = addr_bit_len(DevAddrLegacy);
        assert_eq!(24, Width0);
        let Width1 = addr_bit_len(DevAddr1);
        assert_eq!(10, Width1);
        let Width2 = addr_bit_len(DevAddr2);
        assert_eq!(17, Width2);

        let NwkAddr0 = nwk_addr(DevAddr00);
        assert_eq!(0, NwkAddr0);
        let NwkAddr1 = nwk_addr(DevAddr01);
        assert_eq!(16, NwkAddr1);
        let NwkAddr2 = nwk_addr(DevAddr02);
        assert_eq!(8, NwkAddr2);

        // Backwards DevAddr compatibility test
        // DevAddr00 is a legacy Helium Devaddr.  The NetID is retired.
        // By design we do compute a proper subnet (giving us a correct OUI route),
        // but if we compute the associated DevAddr for this subnet (for the Join request)
        // we'll get a new one associated with a current and proper NetID
        // In other words, DevAddr00 is not equal to DevAddr000.
        let Subnet0 = subnet_from_devaddr(DevAddr00, &NetIDList);
        assert_eq!(None, Subnet0);
        let DevAddr000 = devaddr_from_subnet(0, &NetIDList);
        // By design the reverse DevAddr will have a correct NetID
        assert_ne!(DevAddr000.unwrap(), DevAddr00);
        assert_eq!(Some(0xFE000080), DevAddr000);
        let DevAddr000NetID = parse_netid(DevAddr000.unwrap());
        assert_eq!(NetID00, DevAddr000NetID);

        let Subnet1 = subnet_from_devaddr(DevAddr01, &NetIDList);
        assert_eq!((1 << 7) + 16, Subnet1.unwrap());
        let DevAddr001 = devaddr_from_subnet(Subnet1.unwrap(), &NetIDList);
        assert_eq!(DevAddr001.unwrap(), DevAddr01);

        let Subnet1 = subnet_from_devaddr(DevAddr01, &NetIDList);
        assert_eq!((1 << 7) + 16, Subnet1.unwrap());
        let DevAddr001 = devaddr_from_subnet(Subnet1.unwrap(), &NetIDList);
        assert_eq!(DevAddr001.unwrap(), DevAddr01);

        let Subnet2 = subnet_from_devaddr(DevAddr02, &NetIDList);
        assert_eq!((1 << 7) + (1 << 10) + 8, Subnet2.unwrap());
        let DevAddr002 = devaddr_from_subnet(Subnet2.unwrap(), &NetIDList);
        assert_eq!(DevAddr002.unwrap(), DevAddr02);
    }

    #[test]
    fn test_id() {
        // CP data (matches Erlang test cases)
        // <<91, 255, 255, 255>> "[45] == 2D == 45 type 0"
        assert_eq!(NetId::from(0x00002D), NetId::from(0x5BFFFFFF));
        // <<173, 255, 255, 255>> "[45] == 2D == 45 type 1"
        assert_eq!(NetId::from(0x20002D), NetId::from(0xADFFFFFF));
        // <<214, 223, 255, 255>> "[1,109] == 16D == 365 type 2"
        assert_eq!(NetId::from(0x40016D), NetId::from(0xD6DFFFFF));
        // <<235, 111, 255, 255>>), "[5,183] == 5B7 == 1463 type 3"
        assert_eq!(NetId::from(0x6005B7), NetId::from(0xEB6FFFFF));
        // <<245, 182, 255, 255>>), "[11, 109] == B6D == 2925 type 4"
        assert_eq!(NetId::from(0x800B6D), NetId::from(0xF5B6FFFF));
        // println!(
        //     "left: {:#04X?} right: {:#04X?}",
        //     0xA016DB,
        //     parse_netid(0xFADB7FFF)
        // );
        // <<250, 219, 127, 255>>), "[22,219] == 16DB == 5851 type 5"
        assert_eq!(NetId::from(0xA016DB), NetId::from(0xFADB7FFF));
        // <<253, 109, 183, 255>> "[91, 109] == 5B6D == 23405 type 6"
        assert_eq!(NetId::from(0xC05B6D), NetId::from(0xFD6DB7FF));
        // <<254, 182, 219, 127>> "[1,109,182] == 16DB6 == 93622 type 7"
        assert_eq!(NetId::from(0xE16DB6), NetId::from(0xFEB6DB7F));
        println!(
            "left: {:#04X?} right: {:#04X?}",
            NetId::from(0xA016DB),
            NetId::from(0xFFFFFFFF)
        );
        // FixME - Invalid NetID type
        assert_eq!(NetId::from(127), NetId::from(0xFFFFFFFF));

        // Actility spreadsheet examples
        assert_eq!(NetId::from(0), NetId::from(0));
        assert_eq!(NetId::from(1), NetId::from(1 << 25));
        assert_eq!(NetId::from(2), NetId::from(1 << 26));

        // Mis-parsed as netid 4 of type 3
        assert_eq!(NetId::from(0x600004), NetId::from(0xE009ABCD));
        // Valid DevAddr, NetID not assigned
        assert_eq!(NetId::from(0x20002D), NetId::from(0xADFFFFFF));
        // Less than 32 bit number
        assert_eq!(NetId::from(0), NetId::from(46377));

        // Louis test data
        assert_eq!(NetId::from(0x600002), NetId::from(0xE0040001));
        assert_eq!(NetId::from(0x600002), NetId::from(0xE0052784));
        assert_eq!(NetId::from(0x000002), NetId::from(0x0410BEA3));
    }
}
