//! Les états sauvegardés (« save states ») : une photo complète de la console à un instant donné.
//!
//! Le format est simple : des valeurs écrites à la suite, dans l'ordre. Chaque composant (processeur, écran,
//! son, cartouche…) sait écrire et relire ses propres champs. Un numéro de version en tête permet de refuser
//! proprement un état écrit par une version d'emuland dont le format a changé.

pub struct StateWriter {
    buf: Vec<u8>,
}

impl StateWriter {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn finish(self) -> Vec<u8> {
        self.buf
    }

    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn bool(&mut self, v: bool) {
        self.buf.push(v as u8);
    }
    /// Une suite d'octets précédée de sa longueur.
    pub fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.buf.extend_from_slice(v);
    }
}

impl Default for StateWriter {
    fn default() -> Self {
        Self::new()
    }
}

pub struct StateReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> StateReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len()).ok_or("the state is truncated or damaged")?;
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    pub fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn bool(&mut self) -> Result<bool, String> {
        Ok(self.u8()? != 0)
    }

    /// Relit une suite d'octets et exige qu'elle ait exactement la taille de `out`.
    pub fn bytes_into(&mut self, out: &mut [u8]) -> Result<(), String> {
        let len = self.u32()? as usize;
        if len != out.len() {
            return Err(format!("unexpected size in the state ({len} instead of {})", out.len()));
        }
        out.copy_from_slice(self.take(len)?);
        Ok(())
    }

    /// Relit une suite d'octets de longueur quelconque (limitée à 16 Mio pour ne pas allouer n'importe quoi).
    pub fn vec(&mut self) -> Result<Vec<u8>, String> {
        let len = self.u32()? as usize;
        if len > 16 * 1024 * 1024 {
            return Err("the state is damaged (absurd length)".into());
        }
        Ok(self.take(len)?.to_vec())
    }

    /// Vrai si tout a été lu (un état trop long est suspect).
    pub fn is_finished(&self) -> bool {
        self.pos == self.data.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip_in_order() {
        let mut w = StateWriter::new();
        w.u8(7);
        w.u16(0xBEEF);
        w.u32(123_456);
        w.u64(u64::MAX - 1);
        w.bool(true);
        w.bytes(&[1, 2, 3]);
        let data = w.finish();
        let mut r = StateReader::new(&data);
        assert_eq!(r.u8().unwrap(), 7);
        assert_eq!(r.u16().unwrap(), 0xBEEF);
        assert_eq!(r.u32().unwrap(), 123_456);
        assert_eq!(r.u64().unwrap(), u64::MAX - 1);
        assert!(r.bool().unwrap());
        let mut out = [0u8; 3];
        r.bytes_into(&mut out).unwrap();
        assert_eq!(out, [1, 2, 3]);
        assert!(r.is_finished());
    }

    #[test]
    fn truncated_or_mismatched_data_is_rejected() {
        let mut r = StateReader::new(&[1, 2]);
        assert!(r.u32().is_err());
        let mut w = StateWriter::new();
        w.bytes(&[1, 2, 3]);
        let data = w.finish();
        let mut wrong = [0u8; 2];
        assert!(StateReader::new(&data).bytes_into(&mut wrong).is_err());
    }
}
