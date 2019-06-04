use crate::{error::{Error,
                    Result},
            member::{MemberList,
                     Membership},
            protocol::{newscast,
                       Message},
            rumor::{Departure,
                    Election,
                    ElectionUpdate,
                    Rumor,
                    RumorStore,
                    Service,
                    ServiceConfig,
                    ServiceFile},
            server::Server};
use byteorder::{ByteOrder,
                LittleEndian};
use habitat_core::fs::AtomicWriter;
use std::{collections::HashMap,
          fs::{File,
               OpenOptions},
          io::{self,
               BufReader,
               BufWriter,
               Read,
               Seek,
               SeekFrom,
               Write},
          path::{Path,
                 PathBuf}};

const HEADER_VERSION: u8 = 2;

// Yay, it's magic number time! 48 below represents the size of the version 1 header, which
// was a struct consisting of 6 fields, each u64. Each u64 is 8 bytes in size, so
// 8 * 6 gives us 48. 64 below represents the size of the version 2 header, which was
// a struct consisting of 7 fields, each u64, plus an additional 8 bytes to store the
// size of the header itself, within the header. (8 * 7) + 8 = 64, which is where that
// number comes from. It's necessary to hard code these numbers because since switching the
// Header to hold a HashMap of MESSAGE_ID -> offset, we can't rely on size_of to give us
// the correct information any more.
const HEADER_VERSION_1_SIZE: usize = 48;
const HEADER_VERSION_2_SIZE: usize = 64;

/// A versioned binary file containing rumors exchanged by the butterfly server which have
/// been periodically persisted to disk.
///
/// The contents of the DatFile can be used to rebuild and resume a butterfly server connection
/// if it has been destroyed or lost.
///
/// * Header Version - 1 byte
/// * Header Body - Variable bytes - see Header
/// * Rumors - Variable bytes
#[derive(Debug)]
pub struct DatFile {
    header:      Header,
    header_size: u64,
    path:        PathBuf,
    reader:      BufReader<File>,
}

impl DatFile {
    pub fn read_or_create(data_path: PathBuf, server: &Server) -> Result<Self> {
        let file = OpenOptions::new().create(true)
                                     .read(true)
                                     .write(true)
                                     .open(&data_path)
                                     .map_err(|err| Error::DatFileIO(data_path.clone(), err))?;
        let size = file.metadata()
                       .map_err(|err| Error::DatFileIO(data_path.clone(), err))?
                       .len();
        let reader = BufReader::new(file);
        let dat_file = DatFile { path: data_path,
                                 header_size: 0,
                                 header: Header::default(),
                                 reader };

        if size == 0 {
            dat_file.write(server)?;
        }

        Ok(dat_file)
    }

    pub fn read(data_path: &Path) -> io::Result<Self> {
        Ok(DatFile { header:      Default::default(),
                     header_size: Default::default(),
                     path:        data_path.to_path_buf(),
                     reader:      BufReader::new(File::open(&data_path)?), })
    }

    pub fn path(&self) -> &Path { &self.path }

    pub fn read_header(&mut self, version: &mut [u8]) -> Result<()> {
        self.reader
            .read_exact(version)
            .map_err(|err| Error::DatFileIO(self.path.clone(), err))?;
        debug!("Header Version: {}", version[0]);
        let (header_size, real_header) =
            Header::from_file(&mut self.reader, version[0]).map_err(|err| {
                                                               Error::DatFileIO(self.path.clone(),
                                                                                err)
                                                           })?;
        self.header = real_header;
        self.header_size = header_size;
        debug!("Header Size: {:?}", self.header_size);
        debug!("Header: {:?}", self.header);

        self.reader
            .seek(SeekFrom::Start(self.member_offset()))
            .map_err(|err| Error::DatFileIO(self.path.clone(), err))?;
        Ok(())
    }

    fn read_and_process<F>(&mut self, offset: u64, mut op: F) -> Result<()>
        where F: FnMut(&mut Vec<u8>) -> Result<()>
    {
        let mut bytes_read = 0;
        let mut size_buf = [0; 8];
        let mut rumor_buf: Vec<u8> = vec![];

        loop {
            if bytes_read >= offset {
                break;
            }
            self.reader
                .read_exact(&mut size_buf)
                .map_err(|err| Error::DatFileIO(self.path.clone(), err))?;
            let rumor_size = LittleEndian::read_u64(&size_buf);
            rumor_buf.resize(rumor_size as usize, 0);
            self.reader
                .read_exact(&mut rumor_buf)
                .map_err(|err| Error::DatFileIO(self.path.clone(), err))?;
            bytes_read += size_buf.len() as u64 + rumor_size;
            op(&mut rumor_buf)?;
        }

        Ok(())
    }

    pub fn read_rumors<T>(&mut self) -> Result<Vec<T>>
        where T: Message<newscast::Rumor>
    {
        let mut rumors = Vec::new();
        let offset = self.header.offset_for_rumor(T::MESSAGE_ID);
        self.read_and_process(offset, |r| {
                rumors.push(T::from_bytes(&r)?);
                Ok(())
            })?;
        Ok(rumors)
    }

    pub fn read_members(&mut self) -> Result<Vec<Membership>> {
        let mut members = Vec::new();
        debug!("Reading membership rumors from {}", self.path().display());
        self.read_and_process(self.header.member_offset(), |r| {
                members.push(Membership::from_bytes(&r)?);
                Ok(())
            })?;
        Ok(members)
    }

    pub fn read_into_mlr(&mut self, server: &Server) -> Result<()> {
        let mut version = [0; 1];

        self.read_header(&mut version)?;

        for Membership { member, health } in self.read_members()? {
            server.insert_member(member, health);
        }

        for service in self.read_rumors::<Service>()? {
            server.insert_service(service);
        }

        for service_config in self.read_rumors::<ServiceConfig>()? {
            server.insert_service_config(service_config);
        }

        for service_file in self.read_rumors::<ServiceFile>()? {
            server.insert_service_file(service_file);
        }

        for election in self.read_rumors::<Election>()? {
            server.insert_election_mlr(election);
        }

        for update_election in self.read_rumors::<ElectionUpdate>()? {
            server.insert_update_election_mlr(update_election);
        }

        if version[0] >= 2 {
            for departure in self.read_rumors::<Departure>()? {
                server.insert_departure(departure);
            }
        }

        Ok(())
    }

    /// # Locking
    /// * `MemberList::entries` (read) This method must not be called while any MemberList::entries
    ///   lock is held.
    pub fn write(&self, server: &Server) -> Result<usize> {
        let mut header = Header::default();
        let w =
            AtomicWriter::new(&self.path).map_err(|err| Error::DatFileIO(self.path.clone(), err))?;
        w.with_writer(|mut f| {
             let mut writer = BufWriter::new(&mut f);
             let header_reserve = vec![0; HEADER_VERSION_2_SIZE];
             writer.write(&[HEADER_VERSION])
                   .map_err(|err| Error::DatFileIO(self.path.clone(), err))?;
             writer.write(&header_reserve)
                   .map_err(|err| Error::DatFileIO(self.path.clone(), err))?;
             header.insert_member_offset(self.write_member_list_mlr(&mut writer,
                                                                    &server.member_list)?);
             header.insert_offset_for_rumor(Service::MESSAGE_ID,
                                            self.write_rumor_store(&mut writer,
                                                                   &server.service_store)?);
             header.insert_offset_for_rumor(ServiceConfig::MESSAGE_ID,
                                            self.write_rumor_store(&mut writer,
                                                                   &server.service_config_store)?);
             header.insert_offset_for_rumor(ServiceFile::MESSAGE_ID,
                                            self.write_rumor_store(&mut writer,
                                                                   &server.service_file_store)?);
             header.insert_offset_for_rumor(Election::MESSAGE_ID,
                                            self.write_rumor_store(&mut writer,
                                                                   &server.election_store)?);
             header.insert_offset_for_rumor(ElectionUpdate::MESSAGE_ID,
                                            self.write_rumor_store(&mut writer,
                                                                   &server.update_store)?);
             header.insert_offset_for_rumor(Departure::MESSAGE_ID,
                                            self.write_rumor_store(&mut writer,
                                                                   &server.departure_store)?);
             writer.seek(SeekFrom::Start(1))?;
             self.write_header(&mut writer, &header)?;
             writer.flush()?;
             Ok(0)
         })
         .map_err(|err| {
             match err {
                 Error::UnknownIOError(e) => Error::DatFileIO(self.path.clone(), e),
                 e => e,
             }
         })
    }

    fn member_offset(&self) -> u64 { 1 + self.header_size }

    fn write_header<W>(&self, writer: &mut W, header: &Header) -> Result<usize>
        where W: Write
    {
        let bytes = header.write_to_bytes().unwrap();
        let total = writer.write(&bytes)
                          .map_err(|err| Error::DatFileIO(self.path.clone(), err))?;
        Ok(total)
    }

    /// # Locking
    /// * `MemberList::entries` (read) This method must not be called while any MemberList::entries
    ///   lock is held.
    fn write_member_list_mlr(&self,
                             writer: &mut impl Write,
                             member_list: &MemberList)
                             -> Result<u64> {
        let mut total = 0;
        member_list.with_memberships_mlr(|membership| {
                       total += self.write_member(writer, &membership)?;
                       Ok(total)
                   })
    }

    fn write_member<W>(&self, writer: &mut W, membership: &Membership) -> Result<u64>
        where W: Write
    {
        let mut total = 0;
        let mut len_buf = [0; 8];
        let bytes = membership.clone().write_to_bytes().unwrap();
        LittleEndian::write_u64(&mut len_buf, bytes.len() as u64);
        total += writer.write(&len_buf)
                       .map_err(|err| Error::DatFileIO(self.path.clone(), err))?
                 as u64;
        total += writer.write(&bytes)
                       .map_err(|err| Error::DatFileIO(self.path.clone(), err))?
                 as u64;
        Ok(total)
    }

    fn write_rumor_store<T, W>(&self, writer: &mut W, store: &RumorStore<T>) -> Result<u64>
        where T: Rumor,
              W: Write
    {
        let mut total = 0;
        for member in store.list
                           .read()
                           .expect("Rumor store lock poisoned")
                           .values()
        {
            for rumor in member.values() {
                total += self.write_rumor(writer, rumor)?;
            }
        }
        Ok(total)
    }

    fn write_rumor<T, W>(&self, writer: &mut W, rumor: &T) -> Result<u64>
        where T: Message<newscast::Rumor>,
              W: Write
    {
        let mut total = 0;
        let mut rumor_len = [0; 8];
        let bytes = rumor.write_to_bytes().unwrap();
        LittleEndian::write_u64(&mut rumor_len, bytes.len() as u64);
        total += writer.write(&rumor_len)
                       .map_err(|err| Error::DatFileIO(self.path.clone(), err))?
                 as u64;
        total += writer.write(&bytes)
                       .map_err(|err| Error::DatFileIO(self.path.clone(), err))?
                 as u64;
        Ok(total)
    }
}

/// Describes contents and structure of dat file.
///
/// The information in this header is used to enable IO seeking operations on a binary dat
/// file containing rumors exchanged by the butterfly server.
#[derive(Debug, Default, PartialEq)]
pub struct Header {
    offsets: HashMap<String, u64>,
}

impl Header {
    pub fn from_file<R>(reader: &mut R, version: u8) -> io::Result<(u64, Self)>
        where R: Read
    {
        let mut bytes = match version {
            1 => vec![0; HEADER_VERSION_1_SIZE],
            _ => vec![0; HEADER_VERSION_2_SIZE],
        };
        reader.read_exact(&mut bytes)?;
        Ok(Self::from_bytes(&bytes, version))
    }

    pub fn insert_member_offset(&mut self, offset: u64) {
        self.offsets
            .insert(Membership::MESSAGE_ID.to_string(), offset);
    }

    pub fn insert_offset_for_rumor(&mut self, message_id: &str, offset: u64) {
        self.offsets.insert(message_id.to_string(), offset);
    }

    pub fn offset_for_rumor(&self, message_id: &str) -> u64 {
        *self.offsets.get(message_id).expect("Rumor offset")
    }

    pub fn member_offset(&self) -> u64 {
        *self.offsets
             .get(Membership::MESSAGE_ID)
             .expect("Membership offset")
    }

    // Returns the size of the struct in bytes *as written*,
    // along with the struct itself future-proofed to the latest version.
    pub fn from_bytes(bytes: &[u8], version: u8) -> (u64, Self) {
        match version {
            // The version 1 header didn't have the size of the header struct itself
            // embedded within it, so we fake it.
            1 => {
                let mut offsets = HashMap::new();
                offsets.insert(Membership::MESSAGE_ID.to_string(),
                               LittleEndian::read_u64(&bytes[0..8]));
                offsets.insert(Service::MESSAGE_ID.to_string(),
                               LittleEndian::read_u64(&bytes[8..16]));
                offsets.insert(ServiceConfig::MESSAGE_ID.to_string(),
                               LittleEndian::read_u64(&bytes[16..24]));
                offsets.insert(ServiceFile::MESSAGE_ID.to_string(),
                               LittleEndian::read_u64(&bytes[24..32]));
                offsets.insert(Election::MESSAGE_ID.to_string(),
                               LittleEndian::read_u64(&bytes[32..40]));
                offsets.insert(ElectionUpdate::MESSAGE_ID.to_string(),
                               LittleEndian::read_u64(&bytes[40..48]));
                offsets.insert(Departure::MESSAGE_ID.to_string(), 0);
                (HEADER_VERSION_1_SIZE as u64, Header { offsets })
            }
            // This should be the latest version of the header. As we deprecate
            // header versions, just roll this code up, and match it, then add
            // your new structure.
            //
            // So copy this struct to the last version number. Then add 8 to the previous struct's
            // (the size of a 64 bit integer) size. Then start the empty fields at 0. The result
            // will be that you read the back-compat version of the data format, and then write the
            // new.
            _ => {
                let mut offsets = HashMap::new();
                offsets.insert(Membership::MESSAGE_ID.to_string(),
                               LittleEndian::read_u64(&bytes[8..16]));
                offsets.insert(Service::MESSAGE_ID.to_string(),
                               LittleEndian::read_u64(&bytes[16..24]));
                offsets.insert(ServiceConfig::MESSAGE_ID.to_string(),
                               LittleEndian::read_u64(&bytes[24..32]));
                offsets.insert(ServiceFile::MESSAGE_ID.to_string(),
                               LittleEndian::read_u64(&bytes[32..40]));
                offsets.insert(Election::MESSAGE_ID.to_string(),
                               LittleEndian::read_u64(&bytes[40..48]));
                offsets.insert(ElectionUpdate::MESSAGE_ID.to_string(),
                               LittleEndian::read_u64(&bytes[48..56]));
                offsets.insert(Departure::MESSAGE_ID.to_string(),
                               LittleEndian::read_u64(&bytes[56..64]));

                (LittleEndian::read_u64(&bytes[0..8]), Header { offsets })
            }
        }
    }

    pub fn write_to_bytes(&self) -> Result<Vec<u8>> {
        // The header is the size of the struct plus 8 bytes for the length of the header itself.
        let header_size = HEADER_VERSION_2_SIZE;
        let mut bytes = vec![0; header_size];
        LittleEndian::write_u64(&mut bytes[0..8], header_size as u64);
        LittleEndian::write_u64(&mut bytes[8..16], self.member_offset());
        LittleEndian::write_u64(&mut bytes[16..24],
                                self.offset_for_rumor(Service::MESSAGE_ID));
        LittleEndian::write_u64(&mut bytes[24..32],
                                self.offset_for_rumor(ServiceConfig::MESSAGE_ID));
        LittleEndian::write_u64(&mut bytes[32..40],
                                self.offset_for_rumor(ServiceFile::MESSAGE_ID));
        LittleEndian::write_u64(&mut bytes[40..48],
                                self.offset_for_rumor(Election::MESSAGE_ID));
        LittleEndian::write_u64(&mut bytes[48..56],
                                self.offset_for_rumor(ElectionUpdate::MESSAGE_ID));
        LittleEndian::write_u64(&mut bytes[56..64],
                                self.offset_for_rumor(Departure::MESSAGE_ID));
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand;
    use std::mem;

    #[test]
    fn read_write_header() {
        // TODO fix this
        assert!(true);
    }
}
