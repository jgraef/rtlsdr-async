use std::io::{
    Error,
    Seek,
    SeekFrom,
    Write,
};

use byteorder::{
    BigEndian,
    WriteBytesExt,
};
use serde::Serialize;

use crate::spatial::in_memory::{
    Node,
    Tree,
};

const MAGIC: &'static [u8] = b"adsb-index-spatial      ";

#[derive(Debug)]
pub struct TreeWriter<W> {
    writer: W,
}

impl<W: Write + Seek> TreeWriter<W> {
    pub fn write_tree(&mut self, _tree: &Tree) -> Result<(), Error> {
        todo!();
    }

    pub fn write_tree_with_metdata<M: Serialize>(
        &mut self,
        _tree: &Tree,
        _metadata: &M,
    ) -> Result<(), Error> {
        todo!();
    }

    fn write_header(&mut self) -> Result<HeaderPlaceholders, Error> {
        // magic - 24 bytes
        self.writer.write_all(MAGIC)?;

        // version - 4 bytes
        self.writer.write_u32::<BigEndian>(1)?;

        // reserved - 4 bytes
        self.writer.write_u32::<BigEndian>(0)?;

        // root node offset - 8 bytes
        let root_offset = Placeholder::create(&mut self.writer)?;

        // reserved (for metadata offset) - 8 bytes
        let metadata_offset = Placeholder::create(&mut self.writer)?;

        // reserved (for metadata length) - 8 bytes
        let metadata_length = Placeholder::create(&mut self.writer)?;

        // padding
        self.writer.write_u64::<BigEndian>(0)?;

        Ok(HeaderPlaceholders {
            root_offset,
            metadata_offset,
            metadata_length,
        })
    }

    fn write_metadata<M: Serialize>(
        &mut self,
        metadata: &M,
        offset_placeholder: Option<Placeholder>,
        length_placeholder: Option<Placeholder>,
    ) -> Result<(), Error> {
        let start_position = self.writer.stream_position()?;
        serde_json::to_writer(&mut self.writer, metadata)?;
        let end_position = self.writer.stream_position()?;

        if let Some(offset_placeholder) = offset_placeholder {
            offset_placeholder.fill_value(&mut self.writer, start_position)?;
        }

        if let Some(length_placeholder) = length_placeholder {
            length_placeholder.fill_value(
                &mut self.writer,
                end_position.checked_sub(start_position).unwrap(),
            )?;
        }

        self.writer
            .seek(SeekFrom::Start(align_offset(end_position, 8)))?;

        Ok(())
    }

    fn write_node(
        &mut self,
        node: &Node,
        offset_placeholder: Option<Placeholder>,
    ) -> Result<(), Error> {
        if node.entries.is_empty() {
            return Ok(());
        }

        let start_position = self.writer.stream_position()?;

        // write children references
        let children_placeholders = [
            Placeholder::create(&mut self.writer)?,
            Placeholder::create(&mut self.writer)?,
            Placeholder::create(&mut self.writer)?,
            Placeholder::create(&mut self.writer)?,
        ];

        // write number of ICAO addresses
        self.writer.write_u32::<BigEndian>(
            node.entries
                .len()
                .try_into()
                .expect("entries.len() can't be converted to u32"),
        )?;

        // write ICAO addresses
        for icao_address in &node.entries {
            self.writer.write_all(&icao_address.as_bytes())?;
        }

        let end_position = self.writer.stream_position()?;

        if let Some(offset_placeholder) = offset_placeholder {
            offset_placeholder.fill_value(&mut self.writer, start_position)?;
        }

        self.writer
            .seek(SeekFrom::Start(align_offset(end_position, 8)))?;

        // write children nodes
        if let Some(children) = &node.children {
            self.write_node(&children[0], Some(children_placeholders[0]))?;
            self.write_node(&children[1], Some(children_placeholders[1]))?;
            self.write_node(&children[2], Some(children_placeholders[2]))?;
            self.write_node(&children[3], Some(children_placeholders[3]))?;
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct Placeholder {
    position: u64,
}

impl Placeholder {
    pub fn create<W: Write + Seek>(mut writer: W) -> Result<Self, Error> {
        let position = writer.stream_position()?;
        writer.write_u64::<BigEndian>(0)?;
        Ok(Self { position })
    }

    pub fn fill_value<W: Write + Seek>(&self, mut writer: W, value: u64) -> Result<(), Error> {
        writer.seek(SeekFrom::Start(self.position))?;
        writer.write_u64::<BigEndian>(value)?;
        Ok(())
    }

    pub fn fill_value_and_return<W: Write + Seek>(
        &self,
        mut writer: W,
        value: u64,
    ) -> Result<(), Error> {
        let position = writer.seek(SeekFrom::Start(self.position))?;
        writer.write_u64::<BigEndian>(value)?;
        writer.seek(SeekFrom::Start(position))?;
        Ok(())
    }

    pub fn fill_current_position<W: Write + Seek>(&self, mut writer: W) -> Result<u64, Error> {
        let position = writer.seek(SeekFrom::Start(self.position))?;
        writer.write_u64::<BigEndian>(position)?;
        writer.seek(SeekFrom::Start(position))?;
        Ok(position)
    }
}

#[derive(Clone, Copy, Debug)]
struct HeaderPlaceholders {
    root_offset: Placeholder,
    metadata_offset: Placeholder,
    metadata_length: Placeholder,
}

#[inline(always)]
const fn align_offset(offset: u64, alignment: u64) -> u64 {
    let align_mask = alignment - 1;
    (offset + align_mask) & !align_mask
}
