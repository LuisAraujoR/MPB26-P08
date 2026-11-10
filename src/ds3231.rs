#![allow(dead_code)]

use core::fmt;
use std::cell::RefCell;

use esp_idf_svc::hal::delay::BLOCK;
use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::sys::EspError;

/// Dirección I2C de 7 bits del RTC DS3231M
pub const DS3231_ADDR: u8 = 0x68;

/// Registros internos del DS3231M
const REG_SECONDS: u8 = 0x00;
const REG_ALARM1_SEC: u8 = 0x07;
const REG_ALARM2_MIN: u8 = 0x0B;
const REG_CONTROL: u8 = 0x0E;
const REG_STATUS: u8 = 0x0F;
const REG_TEMP_MSB: u8 = 0x11;

/// Estructura que representa la fecha y hora del RTC
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTime {
    pub year: u16,       // 2000..=2099
    pub month: u8,       // 1..=12
    pub day: u8,         // 1..=31
    pub day_of_week: u8, // 1..=7 (1 = Lunes, ..., 7 = Domingo)
    pub hour: u8,        // 0..=23 (formato 24 horas)
    pub minute: u8,      // 0..=59
    pub second: u8,      // 0..=59
}

impl DateTime {
    pub fn new(
        year: u16,
        month: u8,
        day: u8,
        day_of_week: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Self {
        Self {
            year,
            month,
            day,
            day_of_week,
            hour,
            minute,
            second,
        }
    }

    pub fn day_name(&self) -> &'static str {
        match self.day_of_week {
            1 => "Lun",
            2 => "Mar",
            3 => "Mie",
            4 => "Jue",
            5 => "Vie",
            6 => "Sab",
            7 => "Dom",
            _ => "---",
        }
    }

    /// Serializa `DateTime` en un arreglo de 8 bytes (ideal para guardar en páginas de EEPROM).
    pub fn to_bytes(&self) -> [u8; 8] {
        let [yh, yl] = self.year.to_be_bytes();
        [
            yh,
            yl,
            self.month,
            self.day,
            self.day_of_week,
            self.hour,
            self.minute,
            self.second,
        ]
    }

    /// Deserializa un `DateTime` desde 8 bytes. Devuelve `None` si los datos no son válidos
    /// (por ejemplo, si la EEPROM está virgen con `0xFF`).
    pub fn from_bytes(b: &[u8; 8]) -> Option<Self> {
        let year = u16::from_be_bytes([b[0], b[1]]);
        let month = b[2];
        let day = b[3];
        let day_of_week = b[4];
        let hour = b[5];
        let minute = b[6];
        let second = b[7];

        if !(2000..=2199).contains(&year)
            || !(1..=12).contains(&month)
            || !(1..=31).contains(&day)
            || !(1..=7).contains(&day_of_week)
            || hour > 23
            || minute > 59
            || second > 59
        {
            return None;
        }

        Some(Self {
            year,
            month,
            day,
            day_of_week,
            hour,
            minute,
            second,
        })
    }
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.day_name(),
            self.year,
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second
        )
    }
}

/// Driver para el RTC DS3231M sobre I2C compartido
pub struct Ds3231<'a, 'd> {
    i2c: &'a RefCell<I2cDriver<'d>>,
}

impl<'a, 'd> Ds3231<'a, 'd> {
    /// Crea una nueva instancia del driver DS3231M compartiendo el bus `I2cDriver`.
    pub fn new(i2c: &'a RefCell<I2cDriver<'d>>) -> Self {
        Self { i2c }
    }

    /// Lee la fecha y hora actual del DS3231M.
    pub fn get_datetime(&mut self) -> Result<DateTime, EspError> {
        let mut buf = [0u8; 7];
        self.i2c
            .borrow_mut()
            .write_read(DS3231_ADDR, &[REG_SECONDS], &mut buf, BLOCK)?;

        let second = bcd_to_dec(buf[0] & 0x7F);
        let minute = bcd_to_dec(buf[1] & 0x7F);

        // Manejo de modo 12h o 24h (bit 6 del registro de horas)
        let hour = if (buf[2] & 0x40) != 0 {
            // Modo 12 horas
            let mut h = bcd_to_dec(buf[2] & 0x1F);
            let is_pm = (buf[2] & 0x20) != 0;
            if h == 12 {
                h = 0;
            }
            if is_pm {
                h += 12;
            }
            h
        } else {
            // Modo 24 horas
            bcd_to_dec(buf[2] & 0x3F)
        };

        let day_of_week = bcd_to_dec(buf[3] & 0x07);
        let day = bcd_to_dec(buf[4] & 0x3F);
        let month = bcd_to_dec(buf[5] & 0x1F);
        let century = if (buf[5] & 0x80) != 0 { 2100u16 } else { 2000u16 };
        let year = century + bcd_to_dec(buf[6]) as u16;

        Ok(DateTime {
            year,
            month,
            day,
            day_of_week,
            hour,
            minute,
            second,
        })
    }

    /// Configura la fecha y hora en el DS3231M (en formato 24 horas) y limpia el bit OSF.
    pub fn set_datetime(&mut self, dt: &DateTime) -> Result<(), EspError> {
        let year_short = (dt.year % 100) as u8;
        let century_bit = if dt.year >= 2100 { 0x80 } else { 0x00 };

        let data = [
            REG_SECONDS,
            dec_to_bcd(dt.second),
            dec_to_bcd(dt.minute),
            dec_to_bcd(dt.hour) & 0x3F, // Bit 6 = 0 para modo 24h
            dec_to_bcd(dt.day_of_week),
            dec_to_bcd(dt.day),
            dec_to_bcd(dt.month) | century_bit,
            dec_to_bcd(year_short),
        ];

        let mut i2c = self.i2c.borrow_mut();
        i2c.write(DS3231_ADDR, &data, BLOCK)?;

        // Limpiar la bandera OSF (Oscillator Stop Flag, bit 7) en el registro de estado (0x0F)
        let mut status = [0u8; 1];
        i2c.write_read(DS3231_ADDR, &[REG_STATUS], &mut status, BLOCK)?;
        i2c.write(DS3231_ADDR, &[REG_STATUS, status[0] & 0x7F], BLOCK)?;

        Ok(())
    }

    /// Verifica si el oscilador se detuvo (por ejemplo, si perdió alimentación de respaldo).
    pub fn lost_power(&mut self) -> Result<bool, EspError> {
        let mut status = [0u8; 1];
        self.i2c
            .borrow_mut()
            .write_read(DS3231_ADDR, &[REG_STATUS], &mut status, BLOCK)?;
        Ok((status[0] & 0x80) != 0)
    }

    /// Lee la temperatura interna del sensor del DS3231M en grados Celsius (°C).
    pub fn get_temperature(&mut self) -> Result<f32, EspError> {
        let mut buf = [0u8; 2];
        self.i2c
            .borrow_mut()
            .write_read(DS3231_ADDR, &[REG_TEMP_MSB], &mut buf, BLOCK)?;

        let msb = buf[0] as i8 as f32;
        let frac = ((buf[1] >> 6) as f32) * 0.25;
        if msb < 0.0 {
            Ok(msb - frac)
        } else {
            Ok(msb + frac)
        }
    }

    /// Habilita o deshabilita la salida de onda cuadrada de 1 Hz en el pin INT/SQW.
    /// (En el DS3231M la frecuencia SQW es fija a 1 Hz cuando INTCN = 0).
    pub fn enable_sqw_1hz(&mut self, enable: bool) -> Result<(), EspError> {
        let mut ctrl = [0u8; 1];
        let mut i2c = self.i2c.borrow_mut();
        i2c.write_read(DS3231_ADDR, &[REG_CONTROL], &mut ctrl, BLOCK)?;

        let new_ctrl = if enable {
            ctrl[0] & !(1 << 2) // INTCN = 0 -> activa salida 1Hz en SQW
        } else {
            ctrl[0] | (1 << 2) // INTCN = 1 -> modo interrupción de alarma
        };

        i2c.write(DS3231_ADDR, &[REG_CONTROL, new_ctrl], BLOCK)
    }

    /// Configura la Alarma 1 para dispararse diariamente a una `hour:minute:second` específica.
    pub fn set_alarm1_daily(
        &mut self,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Result<(), EspError> {
        // A1M1=0, A1M2=0, A1M3=0, A1M4=1 (coincidencia de horas, minutos y segundos)
        let data = [
            REG_ALARM1_SEC,
            dec_to_bcd(second) & 0x7F,
            dec_to_bcd(minute) & 0x7F,
            dec_to_bcd(hour) & 0x3F,
            0x80, // A1M4 = 1 (ignorar día/fecha)
        ];

        let mut i2c = self.i2c.borrow_mut();
        i2c.write(DS3231_ADDR, &data, BLOCK)?;

        // Limpiar bandera A1F y habilitar interrupción A1IE + INTCN
        let mut status = [0u8; 1];
        i2c.write_read(DS3231_ADDR, &[REG_STATUS], &mut status, BLOCK)?;
        i2c.write(DS3231_ADDR, &[REG_STATUS, status[0] & !0x01], BLOCK)?;

        let mut ctrl = [0u8; 1];
        i2c.write_read(DS3231_ADDR, &[REG_CONTROL], &mut ctrl, BLOCK)?;
        i2c.write(DS3231_ADDR, &[REG_CONTROL, ctrl[0] | 0x05], BLOCK) // INTCN=1, A1IE=1
    }

    /// Configura la Alarma 2 para dispararse diariamente a una `hour:minute` específica.
    pub fn set_alarm2_daily(&mut self, hour: u8, minute: u8) -> Result<(), EspError> {
        // A2M2=0, A2M3=0, A2M4=1 (coincidencia de horas y minutos)
        let data = [
            REG_ALARM2_MIN,
            dec_to_bcd(minute) & 0x7F,
            dec_to_bcd(hour) & 0x3F,
            0x80, // A2M4 = 1 (ignorar día/fecha)
        ];

        let mut i2c = self.i2c.borrow_mut();
        i2c.write(DS3231_ADDR, &data, BLOCK)?;

        // Limpiar bandera A2F y habilitar interrupción A2IE + INTCN
        let mut status = [0u8; 1];
        i2c.write_read(DS3231_ADDR, &[REG_STATUS], &mut status, BLOCK)?;
        i2c.write(DS3231_ADDR, &[REG_STATUS, status[0] & !0x02], BLOCK)?;

        let mut ctrl = [0u8; 1];
        i2c.write_read(DS3231_ADDR, &[REG_CONTROL], &mut ctrl, BLOCK)?;
        i2c.write(DS3231_ADDR, &[REG_CONTROL, ctrl[0] | 0x06], BLOCK) // INTCN=1, A2IE=1
    }

    /// Verifica si se disparó alguna alarma `(alarm1, alarm2)` y limpia las banderas activas.
    pub fn check_and_clear_alarms(&mut self) -> Result<(bool, bool), EspError> {
        let mut status = [0u8; 1];
        let mut i2c = self.i2c.borrow_mut();
        i2c.write_read(DS3231_ADDR, &[REG_STATUS], &mut status, BLOCK)?;

        let a1 = (status[0] & 0x01) != 0;
        let a2 = (status[0] & 0x02) != 0;

        if a1 || a2 {
            i2c.write(DS3231_ADDR, &[REG_STATUS, status[0] & !0x03], BLOCK)?;
        }

        Ok((a1, a2))
    }
}

/// Convierte un byte en formato BCD a decimal
#[inline]
fn bcd_to_dec(bcd: u8) -> u8 {
    ((bcd >> 4) * 10) + (bcd & 0x0F)
}

/// Convierte un byte decimal (0..=99) a formato BCD
#[inline]
fn dec_to_bcd(dec: u8) -> u8 {
    ((dec / 10) << 4) | (dec % 10)
}
