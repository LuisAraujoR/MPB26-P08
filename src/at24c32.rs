#![allow(dead_code)]

use std::cell::RefCell;

use esp_idf_svc::hal::delay::{FreeRtos, BLOCK};
use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::sys::{EspError, ESP_ERR_INVALID_ARG};

use crate::ds3231::DateTime;

/// Dirección I2C base de la memoria AT24C32 (cuando A2=0, A1=0, A0=0)
pub const AT24C32_BASE_ADDR: u8 = 0x50;

/// Dirección I2C por defecto en los módulos RTC DS3231 (A2=1, A1=1, A0=1 por pull-up)
pub const AT24C32_DEFAULT_ADDR: u8 = 0x57;

/// Capacidad total de la AT24C32 en bytes (32 Kbits = 4096 bytes)
pub const AT24C32_CAPACITY: u16 = 4096;

/// Tamaño de página de escritura de la AT24C32 (32 bytes)
pub const AT24C32_PAGE_SIZE: usize = 32;

/// Tiempo de espera tras un ciclo de escritura interna en EEPROM (ms)
const WRITE_CYCLE_DELAY_MS: u32 = 10;

/// Driver para la memoria EEPROM I2C AT24C32 (4 KB)
pub struct At24c32<'a, 'd> {
    i2c: &'a RefCell<I2cDriver<'d>>,
    address: u8,
}

impl<'a, 'd> At24c32<'a, 'd> {
    /// Crea una instancia del driver AT24C32 con la dirección por defecto del módulo (`0x57`).
    pub fn new(i2c: &'a RefCell<I2cDriver<'d>>) -> Self {
        Self {
            i2c,
            address: AT24C32_DEFAULT_ADDR,
        }
    }

    /// Crea una instancia del driver AT24C32 especificando la dirección I2C (`0x50..=0x57`).
    pub fn with_address(i2c: &'a RefCell<I2cDriver<'d>>, address: u8) -> Self {
        Self { i2c, address }
    }

    /// Escribe un solo byte en la dirección de memoria `mem_addr` (`0x0000..=0x0FFF`).
    pub fn write_byte(&mut self, mem_addr: u16, value: u8) -> Result<(), EspError> {
        if mem_addr >= AT24C32_CAPACITY {
            return Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>());
        }

        let packet = [
            ((mem_addr >> 8) & 0x0F) as u8,
            (mem_addr & 0xFF) as u8,
            value,
        ];

        self.i2c
            .borrow_mut()
            .write(self.address, &packet, BLOCK)?;

        // Esperar a que termine el ciclo de programación interna de la EEPROM (~5-10 ms)
        FreeRtos::delay_ms(WRITE_CYCLE_DELAY_MS);
        Ok(())
    }

    /// Lee un solo byte desde la dirección de memoria `mem_addr` (`0x0000..=0x0FFF`).
    pub fn read_byte(&mut self, mem_addr: u16) -> Result<u8, EspError> {
        if mem_addr >= AT24C32_CAPACITY {
            return Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>());
        }

        let addr_bytes = [((mem_addr >> 8) & 0x0F) as u8, (mem_addr & 0xFF) as u8];
        let mut buf = [0u8; 1];

        self.i2c
            .borrow_mut()
            .write_read(self.address, &addr_bytes, &mut buf, BLOCK)?;

        Ok(buf[0])
    }

    /// Escribe hasta 32 bytes dentro de una misma página de la AT24C32.
    /// No debe cruzar el límite de página de 32 bytes.
    pub fn write_page(&mut self, mem_addr: u16, data: &[u8]) -> Result<(), EspError> {
        if data.is_empty() {
            return Ok(());
        }

        let page_offset = (mem_addr as usize) % AT24C32_PAGE_SIZE;
        if page_offset + data.len() > AT24C32_PAGE_SIZE
            || (mem_addr as usize) + data.len() > AT24C32_CAPACITY as usize
        {
            return Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>());
        }

        let mut packet = [0u8; 2 + AT24C32_PAGE_SIZE];
        packet[0] = ((mem_addr >> 8) & 0x0F) as u8;
        packet[1] = (mem_addr & 0xFF) as u8;
        packet[2..2 + data.len()].copy_from_slice(data);

        self.i2c
            .borrow_mut()
            .write(self.address, &packet[..2 + data.len()], BLOCK)?;

        FreeRtos::delay_ms(WRITE_CYCLE_DELAY_MS);
        Ok(())
    }

    /// Escribe un bloque de bytes de longitud arbitraria a partir de `mem_addr`,
    /// dividiendo automáticamente la escritura en límites de página de 32 bytes.
    pub fn write_bytes(&mut self, mut mem_addr: u16, mut data: &[u8]) -> Result<(), EspError> {
        if (mem_addr as usize) + data.len() > AT24C32_CAPACITY as usize {
            return Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>());
        }

        while !data.is_empty() {
            let page_offset = (mem_addr as usize) % AT24C32_PAGE_SIZE;
            let space_in_page = AT24C32_PAGE_SIZE - page_offset;
            let chunk_len = data.len().min(space_in_page);

            self.write_page(mem_addr, &data[..chunk_len])?;

            mem_addr += chunk_len as u16;
            data = &data[chunk_len..];
        }

        Ok(())
    }

    /// Lee una secuencia de bytes desde `mem_addr` y la almacena en `buf`.
    pub fn read_bytes(&mut self, mem_addr: u16, buf: &mut [u8]) -> Result<(), EspError> {
        if buf.is_empty() {
            return Ok(());
        }
        if (mem_addr as usize) + buf.len() > AT24C32_CAPACITY as usize {
            return Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>());
        }

        let addr_bytes = [((mem_addr >> 8) & 0x0F) as u8, (mem_addr & 0xFF) as u8];

        self.i2c
            .borrow_mut()
            .write_read(self.address, &addr_bytes, buf, BLOCK)?;

        Ok(())
    }

    /// Guarda un registro `DateTime` (8 bytes) en la dirección `mem_addr` de la EEPROM.
    pub fn write_datetime(&mut self, mem_addr: u16, dt: &DateTime) -> Result<(), EspError> {
        let bytes = dt.to_bytes();
        self.write_bytes(mem_addr, &bytes)
    }

    /// Lee un registro `DateTime` (8 bytes) desde `mem_addr`.
    /// Devuelve `Ok(Some(DateTime))` si el registro es válido, o `Ok(None)` si esa posición está vacía (`0xFF`).
    pub fn read_datetime(&mut self, mem_addr: u16) -> Result<Option<DateTime>, EspError> {
        let mut buf = [0u8; 8];
        self.read_bytes(mem_addr, &mut buf)?;
        Ok(DateTime::from_bytes(&buf))
    }
}
