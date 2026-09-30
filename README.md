## Requisitos de Sistema

Antes de compilar este projeto, você precisa instalar a biblioteca de desenvolvimento ALSA:

```bash
sudo apt-get install libasound2-dev
```

### Por que isso é necessário?

Este projeto usa a biblioteca **CPAL** para acesso a áudio, que internamente depende do **ALSA** (Advanced Linux Sound Architecture) no Linux.