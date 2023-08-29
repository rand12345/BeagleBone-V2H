const ws = new WebSocket('ws://10.0.1.177:5555');

ws.onmessage = (event) => {
    const message = JSON.parse(event.data);

    if (message.Data) {
        updateTable(message.Data);
        updateGraph(message.Data);

    } else if (message.Mode) {
        updateRadioButtons(message.Mode);
    }
};
ws.onopen = () => {
    // Send an initial request when the WebSocket connection is open
    const initialRequestMessage = JSON.stringify({ cmd: "GetJson" });
    ws.send(initialRequestMessage);
    // Start sending periodic requests every 5 seconds
    setInterval(() => {
        const periodicRequestMessage = JSON.stringify({ cmd: "GetJson" });
        ws.send(periodicRequestMessage);
    }, 5000);


};

function updateTable(data) {
    const tableBody = document.querySelector('#dataTable tbody');

    const newRow = document.createElement('tr');
    newRow.innerHTML = `
        <td>${data.soc}</td>
        <td>${data.state}</td>
        <td>${data.temp}</td>
        <td>${data.volts * data.amps}</td>
    `;

    tableBody.innerHTML = ''; // Clear existing rows
    tableBody.appendChild(newRow);
}

function updateRadioButtons(mode) {
    const radioButtons = document.querySelectorAll('input[name="mode"]');
    radioButtons.forEach((radio) => {
        radio.checked = radio.value === mode;
    });
}

const radioButtons = document.querySelectorAll('input[name="mode"]');
radioButtons.forEach((radio) => {
    radio.addEventListener('change', () => {
        if (radio.checked) {
            const mode = radio.value;
            const message = { cmd: { SetMode: mode } };
            ws.send(JSON.stringify(message));
        }
    });
});



window.onload = function () {

}

const dataPoints = [];

const chart = new CanvasJS.Chart("chartContainer", {
    theme: "dark2",
    title: {
        text: "Live Data"
    },
    axisX: {
        title: "time",
        gridThickness: 2,
        interval: 2,
        intervalType: "minute",
        valueFormatString: "hh TT K",
        labelAngle: -20
    },
    axisY: {
        title: "Watts"
    },
    data: [{
        type: "line",
        xValueType: "dateTime",
        dataPoints: dataPoints
    }]
});
// updateData();

// Initial Values
var xValue = new Date();
var yValue = 10;
var newDataCount = 6;

function addData(data) {
    // if (newDataCount != 1) {
    //     $.each(data, function (key, value) {
    //         if (key != "amps") { } else {
    //             dataPoints.push({ x: new Date(), y: parseInt(valie * data.volts) });
    //             xValue++;
    //             yValue = parseInt(value[1]);
    //         }
    //     });
    // } else {
    dataPoints.shift();
    // dataPoints.push({ x: xValue, y: parseInt(data.amps) });
    dataPoints.push({ x: new Date(), y: parseInt(data.amps * data.volts) });
    // xValue++;
    yValue = parseInt(data.amps * data.volts);
    // }

    newDataCount = 1;
    chart.render();
    // setTimeout(updateGraph, 1500);
}

function updateGraph(data) {
    $.getJSON("https://canvasjs.com/services/data/datapoints.php?xstart=" + xValue + "&ystart=" + yValue + "&length=" + newDataCount + "type=json", addData(data));
}

// }
